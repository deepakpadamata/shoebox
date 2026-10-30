//! Text in photos (S1-14). Each platform reads text with its own engine (Apple Vision on macOS;
//! others to come); the result is plain text, so engines may differ between devices.
//!
//! Read text goes into `text` shards of its own, told apart from label shards by their `dims`
//! field: [`OCR_MARK`] instead of a label-list version. A photo with a row there has been read,
//! even if no text was found, so nobody reads it again. Only photos likely to hold text are
//! read: screenshots, and photos labelled as documents, receipts, signs, screens and the like.

use crate::encoder::EncodeError;
use crate::labels::Labels;
use shoebox_core::model::ModelId;

/// `dims` of a text shard holding read text (label shards hold the label list's version).
pub const OCR_MARK: u16 = u16::MAX;

/// Announced in an indexer status's models by a device that can read text, so the reading is
/// shared among those devices only.
pub const OCR_ENGINE: ModelId = ModelId(*b"shoebox-ocr-any-engine-v1.......");

/// Reads the text in a photo.
pub trait Ocr: Send + Sync {
    /// Text in the image (`bytes`: JPEG, PNG, HEIC… as stored), lines separated by newlines.
    fn read_text(&self, bytes: &[u8]) -> Result<String, EncodeError>;
}

/// Labels that say a photo probably holds text worth reading.
pub const TEXT_LABELS: &[&str] = &[
    "document",
    "receipt",
    "ticket",
    "passport",
    "id card",
    "driving license",
    "business card",
    "letter",
    "postcard",
    "map",
    "book",
    "notebook",
    "newspaper",
    "magazine",
    "handwriting",
    "whiteboard",
    "blackboard",
    "sign",
    "poster",
    "menu board",
    "menu",
    "calendar",
    "diagram",
    "spreadsheet",
    "screenshot",
    "text message",
    "email",
    "website",
    "computer screen",
    "laptop",
    "phone",
    "tablet",
    "television screen",
    "price tag",
    "label",
    "certificate",
    "contract",
    "form",
    "bank statement",
    "medical report",
    "prescription",
    "recipe",
    "sheet music",
    "book cover",
    "album cover",
    "street sign",
    "road sign",
    "neon sign",
    "greeting card",
    "invitation",
    "handwritten letter",
    "diary",
    "to-do list",
    "shopping list",
    "sticky notes",
    "calculator",
    "qr code",
    "barcode",
];

/// Whether a photo is worth reading: a screenshot, or one of its labels says text.
pub fn wants_text(labels: &Labels, photo_labels: &[(u16, u8)], screenshot: bool) -> bool {
    screenshot
        || photo_labels
            .iter()
            .any(|(i, _)| labels.name(*i).is_some_and(|n| TEXT_LABELS.contains(&n)))
}

/// A stand-in engine for tests: the text is whatever follows `TEXT:` in the bytes.
pub struct TestOcr;

impl Ocr for TestOcr {
    fn read_text(&self, bytes: &[u8]) -> Result<String, EncodeError> {
        let s = String::from_utf8_lossy(bytes);
        Ok(s.split_once("TEXT:")
            .map(|(_, t)| t.trim().to_string())
            .unwrap_or_default())
    }
}
