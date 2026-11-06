//! The search models, run with `tract` (S1-09): the CLIP text and image encoders behind
//! [`shoebox_search::encoder`]'s traits.
//!
//! Model files live outside the binary (`cargo xtask models`, S1-08) and are loaded lazily.

pub mod clip;
pub mod faces;
pub mod files;
pub mod ocr;

pub use clip::{ClipImage, ClipText};
pub use faces::{SFace, YuNet};
pub use ocr::PpOcr;
