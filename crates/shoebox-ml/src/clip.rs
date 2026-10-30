//! OpenCLIP ViT-B/32 (`laion2b_s34b_b79k`), split into its text and image towers (ONNX, opset
//! 19). Inputs: 77 token ids (int32) or a 1×3×224×224 image; outputs: a 512-d L2-normalised
//! embedding.

use anyhow::{Context, Result};
use shoebox_core::model::ModelId;
use shoebox_search::clip_tokenizer::{ClipTokenizer, CONTEXT};
use shoebox_search::encoder::{normalize, EncodeError, ImageEncoder, Rgb, TextEncoder};
use std::path::Path;
use tract_onnx::prelude::*;

pub const DIMS: usize = 512;
pub const SIZE: u32 = 224;
const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_5, 0.261_302_6, 0.275_777_1];

type Plan = Arc<TypedRunnableModel>;

fn load(path: &Path, input: InferenceFact) -> Result<Plan> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    load_bytes(&bytes, input).with_context(|| format!("loading {}", path.display()))
}

fn load_bytes(bytes: &[u8], input: InferenceFact) -> Result<Plan> {
    let model = tract_onnx::onnx()
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, input)?;
    let model = if std::env::var_os("SHOEBOX_ML_DECLUTTER_ONLY").is_some() {
        model.into_typed()?.into_decluttered()?
    } else {
        model.into_optimized()?
    };
    model.into_runnable()
}

fn err(e: impl std::fmt::Display) -> EncodeError {
    EncodeError(e.to_string())
}

fn output(outputs: TVec<TValue>) -> Result<Vec<f32>, EncodeError> {
    let v = outputs[0].to_plain_array_view::<f32>().map_err(err)?;
    let mut v: Vec<f32> = v.iter().copied().collect();
    normalize(&mut v);
    Ok(v)
}

pub struct ClipText {
    plan: Plan,
    tokenizer: ClipTokenizer,
    model: ModelId,
}

impl ClipText {
    /// The text model this build ships, from the first verified copy in `dirs`
    /// ([`crate::files::search_dirs`]).
    pub fn bundled(dirs: &[std::path::PathBuf]) -> Result<Self> {
        use shoebox_search::models::{CLIP_MODEL, CLIP_TEXT};
        let path = crate::files::locate(&CLIP_TEXT, dirs)
            .with_context(|| format!("{} is not installed", CLIP_TEXT.name))?;
        Self::load(&path, ClipTokenizer::bundled(), CLIP_MODEL)
    }

    pub fn load(path: &Path, tokenizer: ClipTokenizer, model: ModelId) -> Result<Self> {
        let plan = load(path, i32::fact([1, CONTEXT]).into())?;
        Ok(Self {
            plan,
            tokenizer,
            model,
        })
    }
}

impl TextEncoder for ClipText {
    fn model(&self) -> ModelId {
        self.model
    }
    fn dims(&self) -> usize {
        DIMS
    }
    fn encode_text(&self, text: &str) -> Result<Vec<f32>, EncodeError> {
        let ids = self.tokenizer.context(text);
        let input =
            tract_ndarray::Array2::from_shape_vec((1, CONTEXT), ids.to_vec()).map_err(err)?;
        output(
            self.plan
                .run(tvec!(Tensor::from(input).into()))
                .map_err(err)?,
        )
    }
}

pub struct ClipImage {
    plan: Plan,
    model: ModelId,
}

impl ClipImage {
    pub fn load(path: &Path, model: ModelId) -> Result<Self> {
        let plan = load(path, Self::input())?;
        Ok(Self { plan, model })
    }

    /// From the model file's bytes (a phone holds the image model as a blob from a peer).
    pub fn from_bytes(bytes: &[u8], model: ModelId) -> Result<Self> {
        let plan = load_bytes(bytes, Self::input())?;
        Ok(Self { plan, model })
    }

    fn input() -> InferenceFact {
        f32::fact([1, 3, SIZE as usize, SIZE as usize]).into()
    }
}

/// OpenCLIP's preprocessing: shortest side to 224 (bicubic), centre crop, normalise per channel.
pub fn preprocess(image: &Rgb) -> Result<Vec<f32>, EncodeError> {
    let img = image::RgbImage::from_raw(image.width, image.height, image.data.clone())
        .ok_or_else(|| err("image buffer does not match its size"))?;
    let scale = SIZE as f32 / image.width.min(image.height) as f32;
    let (w, h) = (
        ((image.width as f32 * scale).round() as u32).max(SIZE),
        ((image.height as f32 * scale).round() as u32).max(SIZE),
    );
    let resized = image::imageops::resize(&img, w, h, image::imageops::FilterType::CatmullRom);
    let (x0, y0) = ((w - SIZE) / 2, (h - SIZE) / 2);
    let s = SIZE as usize;
    let mut out = vec![0f32; 3 * s * s];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = resized.get_pixel(x0 + x, y0 + y).0;
            for c in 0..3 {
                out[c * s * s + y as usize * s + x as usize] =
                    (f32::from(p[c]) / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    Ok(out)
}

impl ImageEncoder for ClipImage {
    fn model(&self) -> ModelId {
        self.model
    }
    fn dims(&self) -> usize {
        DIMS
    }
    fn encode_image(&self, image: &Rgb) -> Result<Vec<f32>, EncodeError> {
        let s = SIZE as usize;
        let input =
            tract_ndarray::Array4::from_shape_vec((1, 3, s, s), preprocess(image)?).map_err(err)?;
        output(
            self.plan
                .run(tvec!(Tensor::from(input).into()))
                .map_err(err)?,
        )
    }
}
