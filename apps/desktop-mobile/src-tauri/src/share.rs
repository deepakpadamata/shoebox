//! Hand photos to other apps: the core writes the originals out (fetching any that are not on
//! this device), then the platform's own share UI takes the files: the share sheet on Android,
//! the share picker on macOS.

use crate::AppCore;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

#[derive(Deserialize)]
struct Args {
    ids: Vec<String>,
    /// Where the share button is, in CSS pixels of the window: the macOS picker opens next to it.
    anchor: Option<Anchor>,
}

#[derive(Deserialize, Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Anchor {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// The `share_files` command, as the app shell runs it.
pub async fn share_files(
    app: &tauri::AppHandle,
    core: &Arc<AppCore>,
    args: Value,
) -> Result<Value, String> {
    let args: Args = serde_json::from_value(args).map_err(|e| format!("bad arguments: {e}"))?;
    let dest = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("share");
    let paths = core
        .share_files(&args.ids, &dest)
        .await
        .map_err(|e| crate::commands::user_message(&e))?;
    show(app, paths, args.anchor).await?;
    Ok(json!({}))
}

#[cfg(target_os = "android")]
async fn show(
    app: &tauri::AppHandle,
    paths: Vec<PathBuf>,
    _anchor: Option<Anchor>,
) -> Result<(), String> {
    let handle = app.state::<crate::android_photos::PhotoLibrary>().0.clone();
    let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    tokio::task::spawn_blocking(move || {
        handle.run_mobile_plugin::<Value>("shareFiles", json!({ "paths": paths }))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
async fn show(
    app: &tauri::AppHandle,
    paths: Vec<PathBuf>,
    anchor: Option<Anchor>,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("the Shoebox window is closed")?;
    app.run_on_main_thread(move || {
        if let Err(e) = macos::show_picker(&window, &paths, anchor) {
            tracing::warn!("share picker: {e}");
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "android", target_os = "macos")))]
async fn show(
    _app: &tauri::AppHandle,
    _paths: Vec<PathBuf>,
    _anchor: Option<Anchor>,
) -> Result<(), String> {
    Err("Sharing to other apps is not available on this system yet.".into())
}

// AppKit through objc2: a raw window pointer and an Objective-C initialiser.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use super::Anchor;
    use objc2::rc::Retained;
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSURL};
    use std::cell::RefCell;
    use std::path::PathBuf;

    thread_local! {
        // AppKit does not keep the picker alive while it is open.
        static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
    }

    pub fn show_picker(
        window: &tauri::WebviewWindow,
        paths: &[PathBuf],
        anchor: Option<Anchor>,
    ) -> Result<(), String> {
        let ptr = window.ns_view().map_err(|e| e.to_string())?;
        // SAFETY: Tauri hands out the window's content view, alive while the window is.
        let view: &NSView = unsafe { &*(ptr as *const NSView) };
        let urls: Vec<Retained<NSURL>> = paths.iter().filter_map(NSURL::from_file_path).collect();
        // SAFETY: an array of URLs is an array of objects; the picker accepts file URLs.
        let items: Retained<NSArray> =
            unsafe { Retained::cast_unchecked(NSArray::from_retained_slice(&urls)) };
        let picker = unsafe {
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
        };
        let bounds = view.bounds();
        let a = anchor.unwrap_or(Anchor {
            x: bounds.size.width / 2.0,
            y: bounds.size.height - 60.0,
            w: 1.0,
            h: 1.0,
        });
        // The web page measures from the top; an unflipped view from the bottom.
        let (y, edge) = if view.isFlipped() {
            (a.y, NSRectEdge::MinY)
        } else {
            (bounds.size.height - a.y - a.h, NSRectEdge::MaxY)
        };
        picker.showRelativeToRect_ofView_preferredEdge(
            NSRect::new(NSPoint::new(a.x, y), NSSize::new(a.w, a.h)),
            view,
            edge,
        );
        PICKER.with(|p| *p.borrow_mut() = Some(picker));
        Ok(())
    }
}
