//! HEIC and AVIF on Android (I1-02): the system's `ImageDecoder`, through the Kotlin `HeifDecoder`
//! object the app registers at start-up (`NativeBridge.registerImageDecoder`). The bitmap it
//! returns is upright and in sRGB; its pixels are read in place with the NDK's bitmap calls.
//!
//! Calls into Java and the NDK are unsafe; like `android_bridge.rs` this module opts out of the
//! `unsafe_code` lint, and every reference is a global one or dies with its attachment.

#![allow(unsafe_code)]

use anyhow::Result;
use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use shoebox_media::{AnalyzeError, DynamicImage, PlatformDecoder, RgbImage};
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

/// `ANDROID_BITMAP_FORMAT_RGBA_8888` and `ANDROID_BITMAP_RESULT_SUCCESS`.
const FORMAT_RGBA_8888: i32 = 1;
const SUCCESS: i32 = 0;

#[repr(C)]
struct BitmapInfo {
    width: u32,
    height: u32,
    stride: u32,
    format: i32,
    flags: u32,
}

#[link(name = "jnigraphics")]
extern "C" {
    fn AndroidBitmap_getInfo(
        env: *mut jni::sys::JNIEnv,
        bitmap: jni::sys::jobject,
        info: *mut BitmapInfo,
    ) -> i32;
    fn AndroidBitmap_lockPixels(
        env: *mut jni::sys::JNIEnv,
        bitmap: jni::sys::jobject,
        pixels: *mut *mut c_void,
    ) -> i32;
    fn AndroidBitmap_unlockPixels(env: *mut jni::sys::JNIEnv, bitmap: jni::sys::jobject) -> i32;
}

/// The VM and the Kotlin `HeifDecoder` object, callable from any thread.
struct Java {
    vm: JavaVM,
    decoder: GlobalRef,
}

static JAVA: OnceLock<Java> = OnceLock::new();
/// One picture at a time: a 50-megapixel one is 200 MB of bitmap and as much again as RGB, and
/// an import analyses several files in parallel.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct AndroidDecoder;

impl PlatformDecoder for AndroidDecoder {
    fn decode(&self, bytes: &[u8]) -> Result<DynamicImage, AnalyzeError> {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        decode(bytes).map_err(|e| match e {
            Failure::Unsupported => AnalyzeError::Unsupported(
                "this phone has no decoder for it (HEIC needs Android 9, AVIF Android 12)".into(),
            ),
            Failure::Broken(why) => AnalyzeError::Decode(why),
        })
    }
}

enum Failure {
    Unsupported,
    Broken(String),
}

impl<E: std::fmt::Display> From<E> for Failure {
    fn from(e: E) -> Self {
        Failure::Broken(format!("{e}"))
    }
}

fn decode(bytes: &[u8]) -> Result<DynamicImage, Failure> {
    let java = JAVA
        .get()
        .ok_or_else(|| Failure::Broken("the image decoder was not registered".into()))?;
    let mut env = java.vm.attach_current_thread()?;
    let array = env.byte_array_from_slice(bytes)?;
    let called = env.call_method(
        &java.decoder,
        "decode",
        "([B)Landroid/graphics/Bitmap;",
        &[JValue::Object(&array)],
    );
    let bitmap = match called {
        Ok(v) => v.l()?,
        Err(_) if env.exception_check()? => {
            let ex = env.exception_occurred()?;
            env.exception_clear()?;
            let message = env
                .call_method(&ex, "getMessage", "()Ljava/lang/String;", &[])
                .ok()
                .and_then(|m| m.l().ok())
                .map(|m| {
                    env.get_string(&m.into())
                        .map(String::from)
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            return Err(Failure::Broken(format!("ImageDecoder: {message}")));
        }
        Err(e) => return Err(e.into()),
    };
    if bitmap.is_null() {
        return Err(Failure::Unsupported);
    }
    let image = read_pixels(&mut env, &bitmap);
    // The pixels are native memory: give them back now, not when the garbage collector gets to it.
    let _ = env.call_method(&bitmap, "recycle", "()V", &[]);
    image
}

fn read_pixels(env: &mut JNIEnv, bitmap: &JObject) -> Result<DynamicImage, Failure> {
    let raw_env = env.get_raw();
    let raw_bitmap = bitmap.as_raw();
    let mut info = BitmapInfo {
        width: 0,
        height: 0,
        stride: 0,
        format: 0,
        flags: 0,
    };
    // SAFETY: `raw_env` belongs to this attached thread and `raw_bitmap` is a live local reference;
    // the pixels are only read between a successful lock and its unlock, on this thread.
    unsafe {
        if AndroidBitmap_getInfo(raw_env, raw_bitmap, &mut info) != SUCCESS {
            return Err(Failure::Broken("could not read the bitmap's size".into()));
        }
        if info.format != FORMAT_RGBA_8888 {
            return Err(Failure::Broken(format!(
                "unexpected bitmap format {}",
                info.format
            )));
        }
        let mut pixels: *mut c_void = std::ptr::null_mut();
        if AndroidBitmap_lockPixels(raw_env, raw_bitmap, &mut pixels) != SUCCESS || pixels.is_null()
        {
            return Err(Failure::Broken("could not read the bitmap's pixels".into()));
        }
        let (width, height, stride) = (
            info.width as usize,
            info.height as usize,
            info.stride as usize,
        );
        let data = std::slice::from_raw_parts(pixels.cast::<u8>(), stride * height);
        let mut rgb = Vec::with_capacity(width * height * 3);
        for row in data.chunks_exact(stride) {
            for px in row[..width * 4].chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
            }
        }
        AndroidBitmap_unlockPixels(raw_env, raw_bitmap);
        RgbImage::from_raw(info.width, info.height, rgb)
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| Failure::Broken("bad pixel buffer".into()))
    }
}

/// `NativeBridge.registerImageDecoder(HeifDecoder)`: from now on HEIC and AVIF are read with it.
#[no_mangle]
pub extern "system" fn Java_org_shoebox_desktop_NativeBridge_registerImageDecoder(
    env: JNIEnv,
    _class: JClass,
    decoder: JObject,
) {
    let registered = (|| -> Result<()> {
        let java = Java {
            vm: env.get_java_vm()?,
            decoder: env.new_global_ref(&decoder)?,
        };
        if JAVA.set(java).is_ok() {
            shoebox_media::set_platform_decoder(Box::new(AndroidDecoder));
        }
        Ok(())
    })();
    if let Err(e) = registered {
        tracing::warn!("could not register the image decoder: {e:#}");
    }
}
