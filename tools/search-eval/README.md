# Search eval (S1-01, S1-12)

How well search by meaning finds the right photo, and how fast, on captioned photos indexed the way a device indexes
them (the 256 px thumbnail).

```bash
python3 -I tools/search-eval/fetch.py 1000
cargo xtask models
cargo run --release -p shoebox-ml --example search_eval -- models/src/clip fixtures-out/search-eval models/build/clip-text-int8.onnx models/build/clip-image-int8.onnx
```

`fetch.py` takes the first N photos of the COCO Karpathy test split (five captions each) into `fixtures-out/search-eval/`.
The photos stay under their owners' licences: they are never committed or shipped. Results are in
`docs/notes/search-models.md`.
