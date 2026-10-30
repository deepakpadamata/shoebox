"""Weight-only int8 for the CLIP encoders (S1-08): the query-side model ships in every build,
so it must stay near 60 MB, but quantising activations too (ONNX Runtime's dynamic quantiser)
moved the vectors too far (cosine 0.85-0.92 to fp32).

Every large MatMul/Gemm weight becomes int8 with one scale per output column, turned back into
float by Cast and Mul on constants (folded once at load). The token table (49,408 x 512) becomes
int8 with one scale per row and is turned back after the Gather, so it stays int8 in memory.

    uv run --no-project --with onnx==1.17.0 --with numpy python -I scripts/quantize_clip.py in.onnx out.onnx
"""
import sys
import numpy as np
import onnx
from onnx import numpy_helper, helper, TensorProto

src, dst = sys.argv[1], sys.argv[2]
m = onnx.load(src)
g = m.graph
inits = {i.name: i for i in g.initializer}
consumers = {}
for n in g.node:
    for i in n.input:
        consumers.setdefault(i, []).append(n)

new_inits, new_nodes, drop = [], [], set()

def q_columns(w):
    scale = np.abs(w).max(axis=0, keepdims=True) / 127.0
    scale[scale == 0] = 1.0
    return np.clip(np.round(w / scale), -127, 127).astype(np.int8), scale.astype(np.float32)

for n in list(g.node):
    if n.op_type not in ("MatMul", "Gemm") or len(n.input) < 2 or n.input[1] not in inits:
        continue
    name = n.input[1]
    w = numpy_helper.to_array(inits[name]).astype(np.float32)
    if w.ndim != 2 or w.size < 4096 or len(consumers[name]) != 1:
        continue
    trans_b = n.op_type == "Gemm" and any(a.name == "transB" and a.i == 1 for a in n.attribute)
    wq, s = q_columns(w.T if trans_b else w)
    if trans_b:
        wq, s = wq.T.copy(), s.T.copy()
    new_inits += [numpy_helper.from_array(wq, name + "_q"), numpy_helper.from_array(s, name + "_s")]
    new_nodes += [
        helper.make_node("Cast", [name + "_q"], [name + "_c"], to=TensorProto.FLOAT),
        helper.make_node("Mul", [name + "_c", name + "_s"], [name + "_dq"]),
    ]
    n.input[1] = name + "_dq"
    drop.add(name)

# The token table: the Gather whose data input is the largest initializer (text tower only).
gathers = [n for n in g.node if n.op_type == "Gather" and n.input[0] in inits
           and numpy_helper.to_array(inits[n.input[0]]).size > 1_000_000]
emb = max(gathers, key=lambda n: numpy_helper.to_array(inits[n.input[0]]).size) if gathers else None
name = emb.input[0] if emb else None
if emb is None:
    kept = [i for i in g.initializer if i.name not in drop]
    del g.initializer[:]
    g.initializer.extend(kept + new_inits)
    nodes = new_nodes + list(g.node)
    del g.node[:]
    g.node.extend(nodes)
    onnx.checker.check_model(m)
    onnx.save(m, dst)
    print(f"{len(drop)} weights quantised")
    sys.exit(0)
w = numpy_helper.to_array(inits[name]).astype(np.float32)
scale = np.abs(w).max(axis=1, keepdims=True) / 127.0
scale[scale == 0] = 1.0
wq = np.clip(np.round(w / scale), -127, 127).astype(np.int8)
new_inits += [numpy_helper.from_array(wq, name + "_q"), numpy_helper.from_array(scale.astype(np.float32), name + "_s")]
out = emb.output[0]
emb.input[0] = name + "_q"
emb.output[0] = out + "_q"
new_nodes += [
    helper.make_node("Gather", [name + "_s", emb.input[1]], [out + "_rs"], axis=0),
    helper.make_node("Cast", [out + "_q"], [out + "_c"], to=TensorProto.FLOAT),
    helper.make_node("Mul", [out + "_c", out + "_rs"], [out]),
]
drop.add(name)

kept = [i for i in g.initializer if i.name not in drop]
del g.initializer[:]
g.initializer.extend(kept + new_inits)
# Put the new nodes first (constants), keeping a valid topological order for the Gather's tail.
order = list(g.node)
pos = order.index(emb)
tail = new_nodes[-3:]
head = new_nodes[:-3]
nodes = head + order[: pos + 1] + tail + order[pos + 1 :]
del g.node[:]
g.node.extend(nodes)
onnx.checker.check_model(m)
onnx.save(m, dst)
print(f"{len(drop)} weights quantised")
