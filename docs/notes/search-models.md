# Search models (S1-01)

Which model does each job in search, under which licence, and on which runtime. The rules are in
`planning/epics/S1-search.md` §4: weights must be redistributable inside an AGPL-3.0 app next to Apache-2.0/MIT
crates; the query side ships in every build; nothing at query time may need another device.

**Status (2026-10-30): chosen and measured on the Mac; the Pi 5 and the S25 are not measured.** The image–text model
runs under `tract` and finds the right photo as well as it does at full precision (table below). Search by meaning is
**go** on desktop and keeper. Phone speed and memory, and a Pi 5 (or a single core of one), are still to measure: they
decide whether a phone indexes at all and how fast a Pi searches, not whether search works.

## Licences of the weights

| Job | Candidate | Weights licence | Redistributable here? |
|---|---|---|---|
| Image–text | Apple MobileCLIP S0/S1/S2/B, MobileCLIP2 | Apple ML Research Model licence: research only, excludes commercial products, revocable | **No** (only the code is MIT) |
| Image–text | Google SigLIP 2 base (patch16, 224/256) | Apache-2.0 | Yes |
| Image–text | OpenCLIP ViT-B/32 `laion2b_s34b_b79k` | MIT | Yes |
| Image–text | OpenAI CLIP ViT-B/32 | MIT | Yes (English, older) |
| Face detection | YuNet (OpenCV zoo, 2023mar / 2026may) | MIT | Yes |
| Face detection | SCRFD (InsightFace) | non-commercial research | **No** |
| Face detection | BlazeFace (MediaPipe) | Apache-2.0 | Yes (TFLite only; weak on groups) |
| Face embedding | SFace (OpenCV zoo) | Apache-2.0 | Yes |
| Face embedding | EdgeFace | weights CC-BY-NC-SA-4.0 | **No** |
| Face embedding | ArcFace / InsightFace buffalo | non-commercial research | **No** |
| OCR | PaddleOCR PP-OCRv5 mobile (det + rec, official ONNX) | Apache-2.0 | Yes |
| OCR | Tesseract + tessdata_fast | Apache-2.0 | Yes (C++, worse on photos) |
| OCR | ML Kit text recognition, bundled | Google ML Kit terms (closed binary) | No: its SDK reports device, install-id and usage data to Google with no developer opt-out (S2-03) |
| OCR | Apple Vision | system framework | Yes (nothing shipped) |

Sources: each model's `LICENSE` on Hugging Face or GitHub, read 2026-10-30 (MobileCLIP: `apple/MobileCLIP2-S0`
LICENSE; SigLIP 2: `google/siglip2-base-patch16-224` card; OpenCLIP: `laion/CLIP-ViT-B-32-laion2B-s34B-b79K` card;
YuNet and SFace: `opencv/opencv_zoo` model folders; InsightFace README; EdgeFace `Idiap/EdgeFace-*` cards; PaddleOCR
repo).

## Choice

| Job | Model | Dims | Sizes at int8 (est.) | Why |
|---|---|---|---|---|
| Image–text | **OpenCLIP ViT-B/32 `laion2b_s34b_b79k`** | 512 | text 64.7 MB, image 96.0 MB (weight-only int8, below) | MIT, and the only redistributable text tower near the 60 MB query-side budget. English only |
| Face detection | **YuNet 2026may** | — | 0.23 MB | MIT, any input size, five landmarks |
| Face embedding | **SFace** | 128 | 9.9 MB | Apache-2.0; aligned from YuNet's landmarks |
| OCR, keeper and desktop | **PP-OCRv5 mobile** det + latin rec | — | ≈ 13 MB | Apache-2.0, ONNX on the same runtime |
| OCR, Android | **PP-OCRv5 mobile** det + latin rec (the same files as the keeper and desktop) | — | ≈ 13 MB | ML Kit would be faster on the phone's NPU but reports usage data to Google, which the owner ruled out on 2026-11-06 (see *Text in photos on a phone*) |
| OCR, macOS | Apple Vision | — | 0 | System |

SigLIP 2 is the better and multilingual model, but its text tower is 283 MB at int8 (its tokenizer has 256k words),
nearly five times the budget; it stays the candidate if the budget is ever raised or a distilled text tower appears.
Epic §13 q3 (multilingual queries) stays open.

## Runtime

**`tract-onnx` (0.23.9, MIT OR Apache-2.0)** on desktop and keeper: pure Rust, so it builds into the static musl
keeper; it implements every op these graphs use (GatherElements, LayerNormalization, attention). Avoid 0.23.0–0.23.4
(security advisory on the ONNX loader). `ort` needs ONNX Runtime built from source for musl and cannot `load-dynamic`
there; `candle` has CLIP and SigLIP but reads safetensors, which would mean a second export path. On Android: ONNX
Runtime Mobile (XNNPACK, NNAPI where it helps) from Kotlin in the background worker.

**8 bits, weights only.** ONNX Runtime's dynamic quantiser (int8 weights *and* activations) moved the text vectors too
far (cosine 0.85–0.92 to full precision) and tract ran it slower (25 ms against 14). `scripts/quantize_clip.py` keeps
the arithmetic in floats and stores each large weight as int8 with one scale per output column; the token table stays
int8 in memory and is turned back after the lookup. Cosine to full precision: 0.9998. The script is deterministic with
its pinned packages, so `cargo xtask models --check` rebuilds the files and compares them with the hashes compiled into
`crates/shoebox-search/src/models.rs`.

**Memory is the cost.** tract turns the int8 weights back into floats when it loads a model and keeps packed copies
for its matrix kernels: the text model takes about 510 MB resident on the Mac (14 MB before loading), the full-precision
one 640 MB. So the text model loads on the first search by meaning and is let go after two minutes without one
(`TEXT_IDLE`), and a phone loads the image model only inside its charging job. Ways to shrink it later: float16
weights where the CPU has fast half-precision maths (Apple silicon, the S25), or keeping the matrix weights int8 at run
time if tract's quantised matrix product gets fast.

**Android runs the same tract code** instead of ONNX Runtime Mobile through Kotlin (the plan's first choice): one code
path, testable here, nothing extra in the APK. NNAPI would be faster for indexing; measure on the S25 before deciding.

## `ModelId`

`ModelId` = BLAKE3 over, in order: each shipped file's BLAKE3 hash, then the preprocessing description
(`CLIP_PREPROCESS` in `crates/shoebox-search/src/models.rs`: thumbnail, resize, crop, normalisation, tokenizer, int8,
L2-normalised). `cargo xtask models` computes it, so a change to any file or to how inputs are prepared is a new model
whose vectors are never compared with the old ones.

## Measurements

Eval set: the first 1,000 photos of the COCO Karpathy test split with their five captions each (5,001 queries),
fetched by `tools/search-eval/fetch.py` into the gitignored `fixtures-out/search-eval/`. Each photo is indexed as a device
indexes it: its 256 px square thumbnail at JPEG quality 80. Run:
`cargo run --release -p shoebox-ml --example search_eval -- models/src/clip fixtures-out/search-eval models/build/clip-text-int8.onnx models/build/clip-image-int8.onnx`.

Recall@k (text → photo: is the captioned photo among the k best of the 1,000):

| Models | Vectors stored as | R@1 | R@5 | R@10 |
|---|---|---|---|---|
| full precision | full precision | 60.7 | 86.5 | 93.7 |
| int8 (shipped) | full precision | 60.8 | 86.5 | 93.7 |
| int8 (shipped) | **vec8** | 60.5 | 86.4 | 93.7 |
| int8 (shipped) | vec1 → best 10% → vec8 | 60.4 | 86.3 | 93.3 |

Speed and memory (tract 0.23.9, release build, one thread; `cargo run --release -p shoebox-ml --example
device_bench -- <models dir> <photos dir>`, run on 2026-10-30; the S25 run through `adb shell`, the device on USB power):

| | Mac (M-series) | nest (Ryzen 5 3600, musl) | Pi 5 | S25 (Snapdragon 8 Elite) |
|---|---|---|---|---|
| Text encoder, ms/query (p50 / p95) | 14 / 15 | 95 / 102 | — | 68 / 69 |
| Image encoder, ms/photo (256 px thumbnail) | 35 | 296 | — | 285–465 |
| Face detection, ms/photo / embedding, ms/face | 15 / 15 | 64 / 63 | — | 78 / 75 |
| Text model resident after loading | 590 MB | 200 MB | — | 174 MB |
| Image model / face models resident | 214 / 44 MB | 140 / 54 MB | — | 117 / 54 MB |
| Scoring 100k photos (vec8, streamed from the file; p50 / p95) | 7.5 / 7.9 ms | 26 / 28 ms | — | 17 / 18 ms |
| Facts query at 100k photos (p50 / p95) | 2.1 / 2.5 ms | 5.4 / 6.5 ms | — | 2.6 / 3.0 ms |

The Mac is 5–10× faster than the other two at the encoders: tract uses Apple's matrix unit there; on x86_64 and
Android it runs its portable SIMD kernels. Pinning the S25 run to the two prime cores (`taskset c0`) changed little.
No Pi 5 was at hand.

The relative cut of meaning results (S1-12; `MEANING_MARGIN`), on the same set:

| Margin below the best score | Captioned photo kept | Photos shown (of 1,000) |
|---|---|---|
| 0.04 | 87.6% | 4.0 |
| 0.06 | 93.9% | 7.2 |
| **0.08** | **97.1%** | **11.9** |
| 0.10 | 98.4% | 18.7 |

## Faces (S1-17)

YuNet (`face_detection_yunet_2023mar`) and SFace (`face_recognition_sface_2021dec`) from OpenCV's model zoo, shipped as
published (SFace's older ONNX form does not suit the weight-only quantiser; 38.7 MB, indexing devices only). Run with
tract and checked against OpenCV 4.10 on COCO photos: the same faces and scores, embeddings at cosine 0.999–1.000.
Detection takes about 15 ms a photo on the Mac. Faces under 40 px or scoring under 0.8 are skipped. Not measured: recall
on a labelled set, same-person accuracy on the owner's photos.

## Text in photos on a phone (S2-03 spike)

Epic S2 makes a phone with no computer or keeper read the text in its own photos. The engine would be PP-OCRv5 from
PaddleOCR on tract, the runtime the other models already use. This section is the spike: does it run, is it as
accurate as PaddleOCR's own pipeline, and how fast is it.

**Why not ML Kit.** Google's ML Kit data disclosure (checked 2026-11-06) lists device information, application
information, install identifiers, performance metrics, event types and error codes sent to Google for every feature, the
bundled models included, "for diagnostics and usage analytics", and describes no way for an app to turn it off. Images
and recognised text are not on the list. The owner's rule is that it is used only if nothing is sent, so the engine is
the open one.

**The files**, from PaddlePaddle's own repositories on Hugging Face at pinned commits (Apache-2.0), fetched and checked
by `scripts/fetch_ocr_models.py` (SHA-256 pins):

| File | Repository, commit | Size |
|---|---|---|
| detection, `PP-OCRv5_mobile_det` | `PaddlePaddle/PP-OCRv5_mobile_det_onnx` @ `e6f4fa85` | 4.8 MB |
| recognition, `latin_PP-OCRv5_mobile_rec` (Latin script: English, French, German, Spanish, …) | `PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx` @ `89d3a50e` | 8.0 MB |
| the recogniser's 836 characters | `inference.yml` of the same | — |

No conversion is needed: Paddle publishes ONNX files. PP-OCRv6 (tiny, small, medium) has since appeared in the same
organisation; the spike kept v5, which is what was decided.

**What the spike does** (`crates/shoebox-ml/examples/ocr_spike.rs`; PaddleOCR's own pre- and post-processing for these
models): detection on the image with its long side set to 960 and both sides a multiple of 32, DB post-processing
(threshold 0.3, box score 0.6, unclip 1.5); recognition of each box at 48 px high and at least 320 wide, greedy CTC.
Two things about tract: the exported graphs carry symbolic shape annotations that clash with a fixed input, so the
model is loaded with `with_ignore_value_info(true)`; and a plan is built for each input shape the first time it is
used (290 ms for the first photo, 1.1 s over all 50 in the run below, three detection shapes), so an engine should keep
a small set of shapes. Not done: rotated boxes (PaddleOCR crops a rotated rectangle; the spike's boxes are axis-aligned)
and turning vertical text.

**Accuracy.** 50 generated images with known text (10 each of receipts, signs, phone screenshots, pages, posters; 509
lines; slightly rotated, JPEG-compressed, with accents), made by `scripts/ocr_spike_eval.py`. Scored by the character
error of the best-matching line read (a line split at a wide gap counts when the pieces are joined). The reference is
RapidOCR, a port of PaddleOCR's pipeline on onnxruntime, running the same two files with PaddleOCR's settings for them;
PaddleOCR itself was not installed.

| | lines | tract: error | lines within 10% | reference: error | lines within 10% |
|---|---|---|---|---|---|
| screenshots | 190 | 0.000 | 100% | 0.000 | 100% |
| pages | 164 | 0.043 | 93% | 0.000 | 100% |
| signs | 20 | 0.035 | 90% | 0.010 | 95% |
| posters | 30 | 0.164 | 60% | 0.129 | 70% |
| receipts | 105 | 0.388 | 40% | 0.285 | 50% |
| **all** | **509** | **0.105** | **83%** | **0.067** | **88%** |

Where the spike is worse than the reference is slightly rotated text (pages, signs), which the rotated crops of the
engine proper would fix. The receipts and posters that fail, fail for both readers: small text on a rotated, noisy
background. These are synthetic images: real photos are not measured.

**Speed**, Apple M4, release build, one thread per model run, per photo with text, plans already built:

| | median | p95 |
|---|---|---|
| detection | 94 ms | 103 ms |
| recognition (6-19 boxes, 9-13 ms each) | 135 ms | 261 ms |
| **both** | **237 ms** | **334 ms** |

**The S25 was not measured: no phone was connected.** The Android binary builds
(`cargo build --release --target aarch64-linux-android -p shoebox-ml --example ocr_spike`, NDK linker) and is ready.
To measure: `adb push` the binary, the three files in `models/build/` and the image folder to `/data/local/tmp`, run it,
and read `det_ms` and `rec_ms`. The other models run 5-13 times slower on the S25 than on this Mac (table under
*Measurements*), which would put a photo with text at roughly 1.2-3 s.

**Verdict on the gate** (tract runs both models; the text agrees with the reference; an S25 photo with text in about a
second or less): the first two hold on the Mac. The third is not known; the estimate is above it by 1-3×. That is
acceptable for work done overnight while charging (only screenshots and photos labelled as documents, signs and the
like are read), but it is not the "about a second" the plan asked for, so the S25 run decides whether S2-03 goes on
as planned or with the faster paths (NNAPI, fewer or smaller boxes, the v6 mobile models).

## Labels (S1-13)

937 labels with synonyms (`crates/shoebox-search/data/labels.txt`), embedded as "a photo of a {label}". A photo keeps
the labels with a share of at least 0.10 of the softmax over all labels (logit scale 100), at most 8: about 1.8 labels
a photo, every photo labelled. On the eval set a caption names 56% of the kept labels; captions leave much out, so this
is a floor (a hand check of 18 photos found the labels right but for a few borderline ones like "traffic").
