//! The Tauri 2 shell: hosts the app core in-process, exposes it to the webview as one
//! `command` plus an `app-event` event stream, and serves blobs on the `shoebox://` scheme.
//! All logic lives in `desktop_mobile::AppCore`; this file only wires it to Tauri.

use crate::commands::{dispatch, user_message};
use crate::keystore::{migrate_file_key, FileStore, SecretStore};
use crate::{AppCore, AppOptions};
use serde_json::Value;
use std::sync::Arc;
use tauri::http::{header, Response, StatusCode};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
#[cfg(desktop)]
use tauri::WindowEvent;
use tauri::{Emitter, Manager, RunEvent};

struct Core(Arc<AppCore>);

/// The app core, waiting for it if the window asked before startup finished. On phones the web
/// view can load and call in while `setup` is still opening the library (the first start also
/// creates the device key), so "not ready yet" must wait, not fail.
async fn core_when_ready(app: &tauri::AppHandle) -> Result<Arc<AppCore>, String> {
    for _ in 0..600 {
        if let Some(core) = app.try_state::<Core>() {
            return Ok(core.0.clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("Shoebox is taking too long to start. Close it and open it again.".into())
}

#[tauri::command]
async fn command(app: tauri::AppHandle, cmd: String, args: Option<Value>) -> Result<Value, String> {
    let core = core_when_ready(&app).await?;
    dispatch(&core, &cmd, args.unwrap_or(Value::Null))
        .await
        .map_err(|e| user_message(&e))
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let spec = value.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    Some((
        a.parse().ok()?,
        if b.is_empty() {
            None
        } else {
            Some(b.parse().ok()?)
        },
    ))
}

/// `shoebox://localhost/blob/<photo id>?kind=thumb|preview|original`
async fn serve_blob(
    core: Arc<AppCore>,
    request: tauri::http::Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let respond = |status: StatusCode, body: &str| {
        Response::builder()
            .status(status)
            .body(body.as_bytes().to_vec())
            .expect("static response")
    };
    let uri = request.uri();
    let Some(id) = uri.path().strip_prefix("/blob/") else {
        return respond(StatusCode::NOT_FOUND, "not found");
    };
    let kind = uri
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("kind=")))
        .unwrap_or("thumb");
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range);
    match core.blob(id, kind, range).await {
        Ok(Some(b)) => {
            let mut resp = Response::builder()
                .header(header::CONTENT_TYPE, &b.content_type)
                .header(header::ETAG, format!("\"{}\"", b.etag))
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                .header(header::ACCEPT_RANGES, "bytes")
                .header("Access-Control-Allow-Origin", "*");
            resp = match b.range {
                Some((first, last)) => resp.status(StatusCode::PARTIAL_CONTENT).header(
                    header::CONTENT_RANGE,
                    format!("bytes {first}-{last}/{}", b.total),
                ),
                None => resp.status(StatusCode::OK),
            };
            resp.body(b.bytes)
                .unwrap_or_else(|_| respond(StatusCode::INTERNAL_SERVER_ERROR, "error"))
        }
        // Not on this device: the core has started fetching it; the UI retries.
        Ok(None) => respond(StatusCode::NOT_FOUND, "not on this device"),
        Err(e) => respond(StatusCode::BAD_REQUEST, &user_message(&e)),
    }
}

/// The platform keystore when there is one (and there is a key from an earlier run to move into
/// it), a file otherwise (with a warning).
fn secret_store(app: &tauri::AppHandle, data_dir: &std::path::Path) -> Arc<dyn SecretStore> {
    #[cfg(target_os = "android")]
    return android_secret_store(app, data_dir);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        desktop_secret_store(data_dir)
    }
}

#[cfg(target_os = "android")]
fn android_secret_store(
    app: &tauri::AppHandle,
    data_dir: &std::path::Path,
) -> Arc<dyn SecretStore> {
    let store: Arc<dyn SecretStore> = Arc::new(crate::android_photos::AndroidKeystore(
        app.state::<crate::android_photos::PhotoLibrary>().0.clone(),
    ));
    // Keys written by earlier builds (a key file under node/, or the file store) move into the
    // keystore, so the device keeps its identity.
    if let Err(e) = migrate_file_key(&data_dir.join("node"), store.as_ref(), "endpoint-key") {
        tracing::warn!("could not move the device key into the keystore: {e:#}");
    }
    let files = FileStore::new(data_dir.join("secrets"));
    if let Err(e) = crate::keystore::migrate_from_files(&files, store.as_ref(), "endpoint-key") {
        tracing::warn!("could not move the device key into the keystore: {e:#}");
    }
    store
}

#[cfg(not(target_os = "android"))]
fn desktop_secret_store(data_dir: &std::path::Path) -> Arc<dyn SecretStore> {
    #[cfg(all(feature = "keyring", desktop))]
    {
        let store = crate::keystore::KeyringStore::new("org.shoebox.desktop");
        if store.available() {
            let store: Arc<dyn SecretStore> = Arc::new(store);
            if let Err(e) = migrate_file_key(&data_dir.join("node"), store.as_ref(), "endpoint-key")
            {
                tracing::warn!("could not move the device key into the keystore: {e:#}");
            }
            return store;
        }
        tracing::warn!("no system keystore available: keeping the device key in a file");
    }
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::new(data_dir.join("secrets")));
    let _ = migrate_file_key(&data_dir.join("node"), store.as_ref(), "endpoint-key");
    store
}

/// Start the app (desktop `main` and the mobile entry point both come here).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    crate::diagnostics::init_logging();

    let builder = tauri::Builder::default();
    #[cfg(target_os = "android")]
    let builder = builder.plugin(crate::android_photos::init());
    // Native file and folder pickers for importing (desktop only).
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    // Security alerts while the window is hidden (R1-06).
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_notification::init());
    // QR scanning when joining a library from a phone (M4-07).
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    let app = builder
        .invoke_handler(tauri::generate_handler![command])
        .register_asynchronous_uri_scheme_protocol("shoebox", |ctx, request, responder| {
            let core = ctx.app_handle().try_state::<Core>().map(|c| c.0.clone());
            tauri::async_runtime::spawn(async move {
                let resp = match core {
                    Some(core) => serve_blob(core, request).await,
                    None => Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(b"starting".to_vec())
                        .expect("static response"),
                };
                responder.respond(resp);
            });
        })
        .setup(|app| {
            #[cfg(mobile)]
            let _ = &app;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            // One app per data directory: a second launch exits (the OS shows the first window).
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(data_dir.join("app.lock"))?;
            if lock.try_lock().is_err() {
                // On Android the holder is a background run in this same process: stop it and wait.
                #[cfg(target_os = "android")]
                {
                    use std::sync::atomic::Ordering;
                    crate::android_bridge::APP_OPENING.store(true, Ordering::SeqCst);
                    let waited = lock.lock();
                    crate::android_bridge::APP_OPENING.store(false, Ordering::SeqCst);
                    waited?;
                }
                #[cfg(not(target_os = "android"))]
                {
                    eprintln!("Shoebox is already running.");
                    std::process::exit(0);
                }
            }
            app.manage(lock);
            let opts = AppOptions {
                secrets: Some(secret_store(app.handle(), &data_dir)),
                ..AppOptions::default()
            };
            let core = tauri::async_runtime::block_on(AppCore::open(&data_dir, opts))?;
            // Tray icon (M3-16): the app keeps syncing with its window closed; Quit ends it.
            #[cfg(desktop)]
            setup_tray(app)?;

            let mut events = core.events();
            let handle = app.handle().clone();
            #[cfg(desktop)]
            let alerts_core = core.clone();
            tauri::async_runtime::spawn(async move {
                let mut syncing = 0i32;
                while let Ok(ev) = events.recv().await {
                    if let crate::types::AppEvent::Sync { phase, .. } = &ev {
                        syncing = if phase == "Started" {
                            syncing + 1
                        } else {
                            (syncing - 1).max(0)
                        };
                        #[cfg(desktop)]
                        if let Some(tray) = handle.tray_by_id("main") {
                            let _ = tray.set_tooltip(Some(if syncing > 0 {
                                "Shoebox: syncing…"
                            } else {
                                "Shoebox: up to date"
                            }));
                        }
                    }
                    // With the window hidden (closed to the tray) nobody sees the banner: show a
                    // system notification for each new security alert instead (R1-06).
                    #[cfg(desktop)]
                    if ev == crate::types::AppEvent::SecurityChanged {
                        let hidden = handle
                            .get_webview_window("main")
                            .is_none_or(|w| !w.is_visible().unwrap_or(true));
                        if hidden {
                            use tauri_plugin_notification::NotificationExt;
                            for alert in alerts_core.take_unnotified_alerts() {
                                let n = crate::notify::security_notification(&alert);
                                let _ = handle
                                    .notification()
                                    .builder()
                                    .title(n.title)
                                    .body(n.body)
                                    .show();
                            }
                        }
                    }
                    let _ = handle.emit("app-event", ev);
                }
            });
            #[cfg(target_os = "android")]
            {
                // `set_photo_source` starts a background task: do it inside the runtime.
                let source = std::sync::Arc::new(crate::android_photos::AndroidPhotoSource(
                    app.state::<crate::android_photos::PhotoLibrary>().0.clone(),
                ));
                let core = core.clone();
                tauri::async_runtime::block_on(async move { core.set_photo_source(source) });
            }
            app.manage(Core(core));
            Ok(())
        })
        .on_window_event(|_window, _event| {
            #[cfg(desktop)]
            let (window, event) = (_window, _event);
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Keep running in the tray so devices keep syncing; Quit in the tray menu exits.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("building the Shoebox app");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(core) = handle.try_state::<Core>() {
                let _ = tauri::async_runtime::block_on(core.0.shutdown());
            }
        }
    });
}

/// Tray icon (M3-16): the app keeps syncing with its window closed; Quit ends it.
#[cfg(desktop)]
fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Shoebox", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Shoebox", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Shoebox")
        .menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "open" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}
