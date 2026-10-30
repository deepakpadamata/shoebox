//! Compare faces found and described here with OpenCV's (a JSON of reference results).
//! `cargo run --release -p shoebox-ml --example face_check -- <yunet> <sface> <reference.json>`

use shoebox_ml::{SFace, YuNet};
use shoebox_search::encoder::Rgb;
use shoebox_search::faces::{FaceDetector, FaceEmbedder};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (det, rec) = (YuNet::load(a[0].as_ref())?, SFace::load(a[1].as_ref())?);
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(&a[2])?)?;
    for (path, faces) in reference.as_object().unwrap() {
        let img = image::open(path)?.to_rgb8();
        let rgb = Rgb {
            width: img.width(),
            height: img.height(),
            data: img.into_raw(),
        };
        let t = std::time::Instant::now();
        let found = det.detect(&rgb).map_err(|e| anyhow::anyhow!("{e}"))?;
        let took = t.elapsed();
        print!("{} ({took:.0?}):", path.rsplit('/').next().unwrap());
        for f in &found {
            let v = rec
                .embed(&rgb, &f.landmarks)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let best = faces
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let full: Vec<f32> = r["full"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_f64().unwrap() as f32)
                        .collect();
                    full.iter().zip(&v).map(|(a, b)| a * b).sum::<f32>()
                })
                .fold(f32::MIN, f32::max);
            let b = f.bbox;
            print!(
                " [{:.3} box {:.0},{:.0},{:.0},{:.0} cos-to-opencv {:.3}]",
                f.score,
                b[0],
                b[1],
                b[2] - b[0],
                b[3] - b[1],
                best
            );
        }
        println!("  (OpenCV: {} faces)", faces.as_array().unwrap().len());
    }
    Ok(())
}
