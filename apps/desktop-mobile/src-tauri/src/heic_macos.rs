//! HEIC and AVIF on macOS (I1-02): the system's ImageIO decodes them, so no decoder is built
//! into the app. The picture comes back upright (the file's rotation applied) and in sRGB, as
//! `shoebox-media` expects from a platform decoder.
//!
//! This is the one place in the app that calls C directly, so it opts out of the `unsafe_code`
//! lint; every pointer is owned by a [`Cf`] that releases it.
#![allow(unsafe_code)]

use shoebox_media::{AnalyzeError, DynamicImage, PlatformDecoder, RgbImage};
use std::ffi::c_void;
use std::ptr;

type CfRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CgPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CgSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CgRect {
    origin: CgPoint,
    size: CgSize,
}

/// `kCFNumberSInt32Type`.
const CF_NUMBER_SINT32: isize = 3;
/// `kCGImageAlphaNoneSkipLast`: R, G, B and an unused byte, 8 bits each.
const ALPHA_NONE_SKIP_LAST: u32 = 5;
/// Larger than any phone sensor; ImageIO does not enlarge a smaller picture to reach it.
const MAX_PIXELS: i32 = 16_384;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CfRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CfRef);
    fn CFDataCreate(allocator: CfRef, bytes: *const u8, length: isize) -> CfRef;
    fn CFNumberCreate(allocator: CfRef, kind: isize, value: *const c_void) -> CfRef;
    fn CFDictionaryCreate(
        allocator: CfRef,
        keys: *const CfRef,
        values: *const CfRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CfRef;
}

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    static kCGImageSourceCreateThumbnailFromImageAlways: CfRef;
    static kCGImageSourceCreateThumbnailWithTransform: CfRef;
    static kCGImageSourceThumbnailMaxPixelSize: CfRef;
    fn CGImageSourceCreateWithData(data: CfRef, options: CfRef) -> CfRef;
    fn CGImageSourceCreateThumbnailAtIndex(source: CfRef, index: usize, options: CfRef) -> CfRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGColorSpaceSRGB: CfRef;
    fn CGColorSpaceCreateWithName(name: CfRef) -> CfRef;
    fn CGImageGetWidth(image: CfRef) -> usize;
    fn CGImageGetHeight(image: CfRef) -> usize;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CfRef,
        bitmap_info: u32,
    ) -> CfRef;
    fn CGContextDrawImage(context: CfRef, rect: CgRect, image: CfRef);
}

/// A Core Foundation object that is released when it goes out of scope.
struct Cf(CfRef);

impl Cf {
    fn new(raw: CfRef, what: &str) -> Result<Cf, AnalyzeError> {
        if raw.is_null() {
            Err(AnalyzeError::Decode(format!("ImageIO: {what} failed")))
        } else {
            Ok(Cf(raw))
        }
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null object this value owns one reference to.
        unsafe { CFRelease(self.0) }
    }
}

pub struct ImageIo;

impl PlatformDecoder for ImageIo {
    fn decode(&self, bytes: &[u8]) -> Result<DynamicImage, AnalyzeError> {
        // SAFETY: every call gets objects made here and kept alive by their `Cf` until the end of
        // the function; the bitmap buffer outlives the context that draws into it.
        unsafe {
            let data = Cf::new(
                CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize),
                "reading the bytes",
            )?;
            let source = Cf::new(
                CGImageSourceCreateWithData(data.0, ptr::null()),
                "opening the file",
            )?;
            let max = MAX_PIXELS;
            let max = Cf::new(
                CFNumberCreate(ptr::null(), CF_NUMBER_SINT32, ptr::from_ref(&max).cast()),
                "making a number",
            )?;
            // A "thumbnail" at the largest size is the whole picture, with the file's rotation
            // applied (a plain `CGImageSourceCreateImageAtIndex` leaves it on its side).
            let keys = [
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceThumbnailMaxPixelSize,
            ];
            let values = [kCFBooleanTrue, kCFBooleanTrue, max.0];
            let options = Cf::new(
                CFDictionaryCreate(
                    ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    keys.len() as isize,
                    ptr::from_ref(&kCFTypeDictionaryKeyCallBacks),
                    ptr::from_ref(&kCFTypeDictionaryValueCallBacks),
                ),
                "making the options",
            )?;
            let image = Cf::new(
                CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.0),
                "decoding (not a picture this Mac can read)",
            )?;
            let (width, height) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
            if width == 0 || height == 0 {
                return Err(AnalyzeError::Decode("ImageIO: empty picture".into()));
            }
            let mut rgbx = vec![0u8; width * height * 4];
            let space = Cf::new(
                CGColorSpaceCreateWithName(kCGColorSpaceSRGB),
                "colour space",
            )?;
            let context = Cf::new(
                CGBitmapContextCreate(
                    rgbx.as_mut_ptr().cast(),
                    width,
                    height,
                    8,
                    width * 4,
                    space.0,
                    ALPHA_NONE_SKIP_LAST,
                ),
                "making a bitmap",
            )?;
            CGContextDrawImage(
                context.0,
                CgRect {
                    origin: CgPoint { x: 0.0, y: 0.0 },
                    size: CgSize {
                        width: width as f64,
                        height: height as f64,
                    },
                },
                image.0,
            );
            drop(context);
            let rgb: Vec<u8> = rgbx
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            RgbImage::from_raw(width as u32, height as u32, rgb)
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(|| AnalyzeError::Decode("ImageIO: bad pixel buffer".into()))
        }
    }
}

/// Use ImageIO for HEIC and AVIF in this process.
pub fn register() {
    shoebox_media::set_platform_decoder(Box::new(ImageIo));
}
