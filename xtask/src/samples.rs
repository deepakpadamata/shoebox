//! The checked-in sample set under `fixtures/samples/`, generated from scratch so the licensing
//! is unambiguous (CC0 / the repository's own license). Formats that need an external encoder
//! (HEIC via `heif-enc`, video via `ffmpeg`) are skipped with a warning when it is missing.

use crate::exifgen::{build_ifd, Entry, ExifSpec, Val};
use crate::jpeg::render_jpeg;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

fn sample_exif(width: u32, height: u32) -> ExifSpec {
    ExifSpec {
        make: "Shoebox".into(),
        model: "Synthetic Camera".into(),
        lens: Some("Synthetic 35mm f/2".into()),
        taken_at_secs: 1_717_243_200, // 2024-06-01 12:00:00 UTC
        gps: Some((48.8584, 2.2945)),
        width,
        height,
        orientation: 1,
    }
}

/// An RGB phone-screenshot-like image (status bar, cards, buttons) as PNG.
pub fn screenshot_png() -> Vec<u8> {
    let (w, h) = (390u32, 844u32);
    let mut px = vec![245u8; (w * h * 3) as usize];
    let mut rect = |x0: u32, y0: u32, x1: u32, y1: u32, rgb: [u8; 3]| {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&rgb);
            }
        }
    };
    rect(0, 0, w, 44, [30, 30, 36]); // status bar
    rect(0, 44, w, 100, [255, 255, 255]); // title bar
    for i in 0..5 {
        let y = 120 + i * 120;
        rect(16, y, w - 16, y + 100, [255, 255, 255]); // card
        rect(28, y + 14, 88, y + 74, [120 + (i * 25) as u8, 150, 220]); // avatar
        rect(104, y + 20, 280, y + 34, [60, 60, 70]); // title text bar
        rect(104, y + 48, 340, y + 58, [170, 170, 180]); // subtitle text bar
    }
    rect(24, 740, w - 24, 796, [10, 132, 255]); // primary button
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(&px)
        .expect("png data");
    out
}

/// A structurally valid DNG (TIFF/EP, 16-bit RGGB CFA) with synthetic sensor data. It is a
/// stand-in for a camera RAW so import/pairing logic has a `.dng` next to a `.jpg`; it is not
/// a real capture.
pub fn synthetic_dng(width: u32, height: u32) -> Vec<u8> {
    let mut sensor = Vec::with_capacity((width * height * 2) as usize);
    for y in 0..height {
        for x in 0..width {
            let v = (((x * 37 + y * 91) % 4096) * 16) as u16;
            sensor.extend(v.to_le_bytes());
        }
    }
    let entries = |data_off: u32| {
        vec![
            Entry {
                tag: 254,
                val: Val::Long(0),
            },
            Entry {
                tag: 256,
                val: Val::Long(width),
            },
            Entry {
                tag: 257,
                val: Val::Long(height),
            },
            Entry {
                tag: 258,
                val: Val::Short(16),
            },
            Entry {
                tag: 259,
                val: Val::Short(1),
            },
            Entry {
                tag: 262,
                val: Val::Short(32803),
            }, // CFA
            Entry {
                tag: 271,
                val: Val::Ascii("Shoebox".into()),
            },
            Entry {
                tag: 272,
                val: Val::Ascii("Synthetic Camera".into()),
            },
            Entry {
                tag: 273,
                val: Val::Long(data_off),
            },
            Entry {
                tag: 277,
                val: Val::Short(1),
            },
            Entry {
                tag: 278,
                val: Val::Long(height),
            },
            Entry {
                tag: 279,
                val: Val::Long(sensor.len() as u32),
            },
            Entry {
                tag: 33421,
                val: Val::Shorts(vec![2, 2]),
            },
            Entry {
                tag: 33422,
                val: Val::Bytes(vec![0, 1, 1, 2]),
            },
            Entry {
                tag: 50706,
                val: Val::Bytes(vec![1, 4, 0, 0]),
            },
            Entry {
                tag: 50707,
                val: Val::Bytes(vec![1, 1, 0, 0]),
            },
            Entry {
                tag: 50708,
                val: Val::Ascii("Shoebox Synthetic Camera".into()),
            },
            Entry {
                tag: 50721,
                val: Val::SRationals(vec![
                    (1000, 1000),
                    (0, 1000),
                    (0, 1000),
                    (0, 1000),
                    (1000, 1000),
                    (0, 1000),
                    (0, 1000),
                    (0, 1000),
                    (1000, 1000),
                ]),
            },
            Entry {
                tag: 50728,
                val: Val::Rationals(vec![(1, 1), (1, 1), (1, 1)]),
            },
            Entry {
                tag: 50778,
                val: Val::Short(21),
            }, // D65
        ]
    };
    let ifd_len = build_ifd(entries(0), 8).len() as u32;
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(build_ifd(entries(8 + ifd_len), 8));
    out.extend(sensor);
    out
}

fn run(cmd: &mut Command, what: &str) -> io::Result<bool> {
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(true),
        Ok(o) => {
            eprintln!(
                "warning: {what} failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            Ok(false)
        }
        Err(e) => {
            eprintln!("warning: skipping {what}: {e}");
            Ok(false)
        }
    }
}

/// Write the sample set into `out`; returns the file names written.
pub fn write_samples(out: &Path) -> io::Result<Vec<String>> {
    fs::create_dir_all(out)?;
    let mut written = vec![];
    let mut put = |name: &str, bytes: &[u8]| -> io::Result<()> {
        fs::write(out.join(name), bytes)?;
        written.push(name.to_string());
        Ok(())
    };
    put(
        "photo-gps.jpg",
        &render_jpeg(&sample_exif(640, 480), 0xA11CE),
    )?;
    // RAW+JPEG pair: same stem, same EXIF capture time.
    put("IMG_0001.jpg", &render_jpeg(&sample_exif(640, 480), 0xB0B))?;
    put("IMG_0001.dng", &synthetic_dng(64, 48))?;
    put("screenshot.png", &screenshot_png())?;

    let jpg = out.join("photo-gps.jpg");
    let heic = out.join("photo.heic");
    if run(
        Command::new("heif-enc").arg(&jpg).arg("-o").arg(&heic),
        "HEIC encode (heif-enc)",
    )? {
        written.push("photo.heic".into());
    }
    let clip = |name: &str, extra: &[&str]| -> io::Result<bool> {
        run(
            Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg("testsrc2=size=320x240:rate=25:duration=2")
                .args([
                    "-c:v", "libx264", "-preset", "veryfast", "-crf", "30", "-pix_fmt", "yuv420p",
                ])
                .args(["-metadata", "creation_time=2024-06-01T12:00:00Z"])
                .args(extra)
                .arg(out.join(name)),
            &format!("video encode (ffmpeg) for {name}"),
        )
    };
    if clip("clip.mp4", &[])? {
        written.push("clip.mp4".into());
    }
    if clip("clip.mov", &["-metadata", "location=+48.8584+002.2945/"])? {
        written.push("clip.mov".into());
    }
    Ok(written)
}
