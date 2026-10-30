//! Text in photos on macOS (S1-14): Apple Vision's text recognition, on this device, from the
//! stored bytes (JPEG, PNG, HEIC). Nothing it reads is logged.

use objc2::rc::autoreleasepool;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};
use shoebox_search::encoder::EncodeError;
use shoebox_search::ocr::Ocr;

pub struct VisionOcr;

impl Ocr for VisionOcr {
    fn read_text(&self, bytes: &[u8]) -> Result<String, EncodeError> {
        autoreleasepool(|_| {
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            request.setAutomaticallyDetectsLanguage(true);
            let data = NSData::with_bytes(bytes);
            let handler = VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &data,
                &NSDictionary::new(),
            );
            let as_request: &VNRequest = &request;
            let requests = NSArray::from_slice(&[as_request]);
            handler.performRequests_error(&requests).map_err(|e| {
                EncodeError(format!(
                    "text recognition failed: {}",
                    e.localizedDescription()
                ))
            })?;
            let mut lines = vec![];
            if let Some(results) = request.results() {
                for obs in results.iter() {
                    if let Some(best) = obs.topCandidates(1).firstObject() {
                        lines.push(best.string().to_string());
                    }
                }
            }
            Ok(lines.join("\n"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_receipt() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../fixtures/ocr/receipt.png"
        ))
        .unwrap();
        let text = VisionOcr.read_text(&bytes).unwrap();
        let folded = text.to_lowercase();
        for word in ["corner shop", "grand total", "12.50", "croissant"] {
            assert!(folded.contains(word), "{word:?} not in {text:?}");
        }
        assert_eq!(
            VisionOcr
                .read_text(b"not an image")
                .ok()
                .as_deref()
                .unwrap_or(""),
            ""
        );
    }
}
