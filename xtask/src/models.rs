//! `cargo xtask models` (S1-08): fetch the search models' source files from pinned URLs, check
//! them, build the files Shoebox ships (weight-only int8, `scripts/quantize_clip.py`) and write
//! their hashes and the `ModelId` into `crates/shoebox-search/src/models.rs`.
//!
//! Everything lands in the gitignored `models/` folder. The build is deterministic with the pinned
//! Python packages, so `--check` (the release workflow) rebuilds and compares against the hashes
//! compiled into the app: a peer's copy of a model is verified like any blob.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file downloaded as is.
pub struct Source {
    pub path: &'static str,
    /// Under [`CLIP_REPO`], or a full URL.
    pub url: &'static str,
    pub blake3: &'static str,
}

/// A file Shoebox ships, made from a source.
pub struct Built {
    /// Name in `models/build/` and in the app's resources.
    pub name: &'static str,
    pub from: &'static str,
    /// The constant's name in the generated `models.rs`.
    pub constant: &'static str,
    /// Weight-only int8 (`scripts/quantize_clip.py`), or shipped as published.
    pub quantize: bool,
    /// The recogniser's character list, one per line, taken from its `inference.yml`
    /// (`scripts/ocr_dict.py`).
    pub dict: bool,
}

/// Where the release workflow publishes the built files (`models/build/*`): one fixed release, so
/// the address does not change with app versions. A phone that finds no model on a device of its
/// own library downloads from here once, and checks the file against the hash compiled in. A change
/// to a model file is a new release (`models-v2`) together with the new hashes.
pub const MODELS_RELEASE: &str =
    "https://github.com/deepakpadamata/shoebox/releases/download/models-v1";

const CLIP_REPO: &str =
    "https://huggingface.co/immich-app/ViT-B-32__laion2b-s34b-b79k/resolve/19a057a0fb927cf239541300e74237cd56c7ffe2";

pub const SOURCES: &[Source] = &[
    Source {
        path: "clip/textual/model.onnx",
        url: "textual/model.onnx",
        blake3: "569eb6a6544b3e647b9b27d124ba7d8d775d8ec18620e2acc0cd23acfc37c8d9",
    },
    Source {
        path: "clip/visual/model.onnx",
        url: "visual/model.onnx",
        blake3: "ce02e121e2aa3a98d6a0e5b0b4e4c75134482d6bc33837dc4cae29266f1c0582",
    },
    // Faces (S1-17), from OpenCV's model zoo: YuNet (MIT) and SFace (Apache-2.0).
    Source {
        path: "faces/face_detection_yunet_2023mar.onnx",
        url: "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
        blake3: "3d5938c4cd5a02dc416f1cd1f7fc1f662a22adc370477112c871954587e63431",
    },
    Source {
        path: "faces/face_recognition_sface_2021dec.onnx",
        url: "https://huggingface.co/opencv/face_recognition_sface/resolve/3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx",
        blake3: "584d0f5792e9e776ab5e5bf3f559f2310ff8fdd3b39f94dd364751046592b512",
    },
    // Text in photos (S2-03): PaddleOCR's PP-OCRv5 mobile detection and Latin recognition
    // (Apache-2.0), the official ONNX files at pinned commits, and the recogniser's config.
    Source {
        path: "ocr/PP-OCRv5_mobile_det.onnx",
        url: "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/e6f4fa85f00e168c862bc462aebca69eef9b3d3d/inference.onnx",
        blake3: "046952fb02f0b6c8825a87e2d35b5a2d38965f2ecaa364148d3e58ecde32cc3f",
    },
    Source {
        path: "ocr/latin_PP-OCRv5_mobile_rec.onnx",
        url: "https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx/resolve/89d3a50e2c27e2e7cceeab0e944c25c807d5db4f/inference.onnx",
        blake3: "80cbd77e85630b3fe029aef7d82cfe50bbcde39440b153defbbfe03c43f4e6d2",
    },
    Source {
        path: "ocr/latin_PP-OCRv5_mobile_rec.yml",
        url: "https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx/resolve/89d3a50e2c27e2e7cceeab0e944c25c807d5db4f/inference.yml",
        blake3: "8b270c12fa650d0a2fdd7f42aeb13fea0bc86ad754529d532f108f9649e64bf4",
    },
];

pub const BUILT: &[Built] = &[
    Built {
        name: "clip-text-int8.onnx",
        from: "clip/textual/model.onnx",
        constant: "CLIP_TEXT",
        quantize: true,
        dict: false,
    },
    Built {
        name: "clip-image-int8.onnx",
        from: "clip/visual/model.onnx",
        constant: "CLIP_IMAGE",
        quantize: true,
        dict: false,
    },
    Built {
        name: "face-detect-yunet.onnx",
        from: "faces/face_detection_yunet_2023mar.onnx",
        constant: "FACE_DETECT",
        quantize: false,
        dict: false,
    },
    Built {
        name: "face-embed-sface.onnx",
        from: "faces/face_recognition_sface_2021dec.onnx",
        constant: "FACE_EMBED",
        quantize: false,
        dict: false,
    },
    Built {
        name: "ocr-det-ppocrv5-mobile.onnx",
        from: "ocr/PP-OCRv5_mobile_det.onnx",
        constant: "OCR_DET",
        quantize: false,
        dict: false,
    },
    Built {
        name: "ocr-rec-ppocrv5-latin.onnx",
        from: "ocr/latin_PP-OCRv5_mobile_rec.onnx",
        constant: "OCR_REC",
        quantize: false,
        dict: false,
    },
    Built {
        name: "ocr-dict-ppocrv5-latin.txt",
        from: "ocr/latin_PP-OCRv5_mobile_rec.yml",
        constant: "OCR_DICT",
        quantize: false,
        dict: true,
    },
];

/// How faces are found and described (part of the face model's id, `FACES_MODEL`).
pub const FACES_PREPROCESS: &str = "yunet-2023mar on the upright preview or original (EXIF orientation applied, at most 2048 px) letterboxed into 640 (BGR, 0-255), score >= 0.8, NMS 0.3, faces >= 40 px; sface-2021dec on the face aligned to the ArcFace 112 template (RGB, 0-255); L2-normalised 128-d";

/// What the image–text model's vectors depend on besides the weights (part of its `ModelId`).
pub const CLIP_PREPROCESS: &str = "openclip-vit-b-32-laion2b_s34b_b79k; image: 256 px square thumbnail, shortest side to 224 (bicubic), centre crop, mean 0.48145466,0.4578275,0.40821073, std 0.26862954,0.26130258,0.27577711; text: CLIP BPE 49408, 77 tokens; weight-only int8; L2-normalised 512-d";

const PY: &[&str] = &[
    "run",
    "--quiet",
    "--no-project",
    "--with",
    "onnx==1.17.0",
    "--with",
    "numpy==2.1.3",
    "python",
    "-I",
];

pub fn blake3_file(path: &Path) -> Result<(String, u64), String> {
    let mut h = blake3::Hasher::new();
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let n = std::io::copy(&mut f, &mut h).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((h.finalize().to_hex().to_string(), n))
}

fn fetch(src: &Source, dir: &Path) -> Result<(), String> {
    let path = dir.join("src").join(src.path);
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let part = path.with_extension("part");
        let url = if src.url.starts_with("https://") {
            src.url.to_string()
        } else {
            format!("{CLIP_REPO}/{}", src.url)
        };
        println!("fetching {url}");
        let ok = Command::new("curl")
            .args(["-sSfL", "--retry", "3", "-o"])
            .arg(&part)
            .arg(&url)
            .status()
            .map_err(|e| format!("curl: {e}"))?
            .success();
        if !ok {
            return Err(format!("could not download {url}"));
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    let (hash, _) = blake3_file(&path)?;
    if hash != src.blake3 {
        return Err(format!(
            "{} has BLAKE3 {hash}, expected {}: delete it to fetch again, or update the pin on purpose",
            path.display(),
            src.blake3
        ));
    }
    Ok(())
}

fn build(b: &Built, dir: &Path, root: &Path) -> Result<PathBuf, String> {
    let out = dir.join("build").join(b.name);
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    if b.dict {
        let ok = Command::new("uv")
            .args([
                "run",
                "--quiet",
                "--no-project",
                "--with",
                "pyyaml==6.0.2",
                "python",
                "-I",
            ])
            .arg(root.join("scripts/ocr_dict.py"))
            .arg(dir.join("src").join(b.from))
            .arg(&out)
            .status()
            .map_err(|e| format!("uv (https://docs.astral.sh/uv/) is needed to build models: {e}"))?
            .success();
        if !ok {
            return Err(format!("building {} failed", b.name));
        }
        return Ok(out);
    }
    if !b.quantize {
        std::fs::copy(dir.join("src").join(b.from), &out).map_err(|e| e.to_string())?;
        return Ok(out);
    }
    println!("building {}", b.name);
    let ok = Command::new("uv")
        .args(PY)
        .arg(root.join("scripts/quantize_clip.py"))
        .arg(dir.join("src").join(b.from))
        .arg(&out)
        .status()
        .map_err(|e| format!("uv (https://docs.astral.sh/uv/) is needed to build models: {e}"))?
        .success();
    if !ok {
        return Err(format!("building {} failed", b.name));
    }
    Ok(out)
}

/// The generated Rust source for the built files.
pub fn render(files: &[(&Built, String, u64)]) -> String {
    let mut model = blake3::Hasher::new();
    let mut out = String::from(
        "//! Generated by `cargo xtask models` (S1-08); do not edit. The search models this build\n\
         //! ships or fetches from peers, by hash: a copy from anywhere is checked against these.\n\n\
         use shoebox_core::model::ModelId;\n\n\
         /// A model file: its name in the app's resources and `models/build/`, BLAKE3, size and the\n\
         /// address of its published copy (the file is checked against the hash whatever served it).\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub struct ModelFile {\n    pub name: &'static str,\n    pub blake3: [u8; 32],\n    pub size: u64,\n    pub url: &'static str,\n}\n",
    );
    let mut faces = blake3::Hasher::new();
    for (b, hash, size) in files {
        let bytes: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&hash[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        // The text reader's files are in neither id: a search index does not depend on them.
        if b.constant.starts_with("FACE") {
            faces.update(&bytes);
        } else if b.constant.starts_with("CLIP") {
            model.update(&bytes);
        }
        let _ = write!(
            out,
            "\npub const {}: ModelFile = ModelFile {{\n    name: {:?},\n    blake3: {:?},\n    size: {size},\n    url: {:?},\n}};\n",
            b.constant,
            b.name,
            bytes,
            format!("{MODELS_RELEASE}/{}", b.name)
        );
    }
    let names: Vec<&str> = files.iter().map(|(b, _, _)| b.constant).collect();
    let _ = write!(
        out,
        "\n/// Every file, for keeping and sharing them.\npub const ALL: &[ModelFile] = &[{}];\n",
        names.join(", ")
    );
    model.update(CLIP_PREPROCESS.as_bytes());
    let id: Vec<u8> = model.finalize().as_bytes().to_vec();
    let _ = write!(
        out,
        "\n/// How the image–text model's inputs are prepared (part of its id).\npub const CLIP_PREPROCESS: &str = {CLIP_PREPROCESS:?};\n\n\
         /// The image–text model: BLAKE3 over its files' hashes, then [`CLIP_PREPROCESS`].\n\
         pub const CLIP_MODEL: ModelId = ModelId({id:?});\n\npub const CLIP_DIMS: usize = 512;\n"
    );
    faces.update(FACES_PREPROCESS.as_bytes());
    let fid: Vec<u8> = faces.finalize().as_bytes().to_vec();
    let _ = write!(
        out,
        "\n/// How faces are found and described (part of their id).\npub const FACES_PREPROCESS: &str = {FACES_PREPROCESS:?};\n\n\
         /// The face models: BLAKE3 over their files' hashes, then [`FACES_PREPROCESS`].\n\
         pub const FACES_MODEL: ModelId = ModelId({fid:?});\n\npub const FACE_DIMS: usize = 128;\n"
    );
    out
}

/// Format generated source as `cargo fmt` would, so the file stays as committed.
fn rustfmt(src: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2021"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("rustfmt: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(src.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("rustfmt failed on the generated models.rs".into());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

pub struct Report {
    pub files: Vec<(String, String, u64)>,
    pub written: Option<PathBuf>,
}

/// Fetch, build and write `models.rs` (or, with `check`, compare against it).
pub fn run(root: &Path, dir: &Path, check: bool) -> Result<Report, String> {
    for s in SOURCES {
        fetch(s, dir)?;
    }
    let mut files = vec![];
    for b in BUILT {
        let path = build(b, dir, root)?;
        let (hash, size) = blake3_file(&path)?;
        files.push((b, hash, size));
    }
    let src = rustfmt(&render(&files))?;
    let target = root.join("crates/shoebox-search/src/models.rs");
    let report = files
        .iter()
        .map(|(b, h, s)| (b.name.to_string(), h.clone(), *s))
        .collect();
    if check {
        let current = std::fs::read_to_string(&target).map_err(|e| e.to_string())?;
        if current != src {
            return Err(format!(
                "the built models do not match {}: run `cargo xtask models` and commit, or fix the build",
                target.display()
            ));
        }
        return Ok(Report {
            files: report,
            written: None,
        });
    }
    std::fs::write(&target, src).map_err(|e| e.to_string())?;
    // The label vectors (S1-13), made with the text model just built.
    println!("embedding the labels");
    let ok = Command::new("cargo")
        .args([
            "run",
            "--release",
            "--quiet",
            "-p",
            "shoebox-ml",
            "--example",
            "label_embeddings",
            "--",
        ])
        .arg(dir.join("build"))
        .arg(root.join("crates/shoebox-search/data/labels.txt"))
        .arg(root.join("crates/shoebox-search/data/labels.bin"))
        .status()
        .map_err(|e| format!("cargo: {e}"))?
        .success();
    if !ok {
        return Err("embedding the labels failed".into());
    }
    Ok(Report {
        files: report,
        written: Some(target),
    })
}
