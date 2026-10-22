//! Headless entry for Android background work (M4-08/09): `BurstWorker` (WorkManager) calls
//! `NativeBridge.runBurst` while the app is closed. This opens the app core without any UI, runs the
//! camera-roll backup and a time-limited `sync_burst`, and returns a JSON report.
//!
//! The camera roll and the Keystore reach Rust through the Kotlin `PhotoBackend` object that the
//! worker passes in (JNI calls), because there is no Activity or Tauri plugin in this mode.

#![allow(unsafe_code)]

use crate::keystore::SecretStore;
use crate::notify::{decide, NotifyInput};
use crate::photos_source::{Asset, Conditions, Permission, PhotoSource, Scan};
use crate::{AppCore, AppOptions};
use anyhow::{anyhow, bail, Result};
use futures::future::BoxFuture;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jint, jlong, jstring};
use jni::{JNIEnv, JavaVM};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The Kotlin `PhotoBackend` object, callable from any thread.
struct Backend {
    vm: JavaVM,
    obj: GlobalRef,
}

impl Backend {
    /// Call a Kotlin method taking only `String?` arguments and returning `String?` (or `void`).
    /// Everything happens inside one attachment: local references die with it.
    fn call(&self, name: &str, sig: &str, args: &[Option<&str>]) -> Result<Option<String>> {
        let mut env = self.vm.attach_current_thread()?;
        let jargs: Vec<JObject> = args
            .iter()
            .map(|a| match a {
                Some(s) => env.new_string(s).map(JObject::from),
                None => Ok(JObject::null()),
            })
            .collect::<std::result::Result<_, _>>()?;
        let values: Vec<JValue> = jargs.iter().map(JValue::Object).collect();
        match env.call_method(&self.obj, name, sig, &values) {
            Ok(v) => match v.l() {
                Ok(o) if o.is_null() => Ok(None),
                Ok(o) => Ok(Some(env.get_string(&JString::from(o))?.into())),
                Err(_) => Ok(None), // a `void` method
            },
            Err(e) => {
                let mut message = e.to_string();
                if env.exception_check()? {
                    let ex = env.exception_occurred()?;
                    env.exception_clear()?;
                    if let Ok(m) = env.call_method(&ex, "getMessage", "()Ljava/lang/String;", &[]) {
                        if let Ok(o) = m.l() {
                            if !o.is_null() {
                                message = env.get_string(&JString::from(o))?.into();
                            }
                        }
                    }
                }
                Err(anyhow!("{name}: {message}"))
            }
        }
    }

    fn string_call(&self, name: &str, sig: &str, arg: Option<&str>) -> Result<String> {
        Ok(self.call(name, sig, &[arg])?.unwrap_or_default())
    }
}

struct JniPhotoSource(Arc<Backend>);

#[derive(Deserialize)]
struct ScanJson {
    assets: Vec<Asset>,
    removed: Vec<String>,
    token: String,
}

#[derive(Deserialize)]
struct ConditionsJson {
    on_wifi: bool,
    charging: bool,
    battery_pct: u8,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}

impl PhotoSource for JniPhotoSource {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>> {
        let b = self.0.clone();
        Box::pin(async move {
            let s = blocking(move || {
                Ok(b.call("permission", "()Ljava/lang/String;", &[])?
                    .unwrap_or_default())
            })
            .await?;
            Ok(match s.as_str() {
                "granted" => Permission::Granted,
                "limited" => Permission::Limited,
                _ => Permission::Denied,
            })
        })
    }

    /// The system prompt needs an Activity: not possible in the background.
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>> {
        self.permission()
    }

    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>> {
        let b = self.0.clone();
        Box::pin(async move {
            let json = blocking(move || {
                b.string_call(
                    "scanJson",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    token.as_deref(),
                )
            })
            .await?;
            let s: ScanJson = serde_json::from_str(&json)?;
            Ok(Scan {
                assets: s.assets,
                removed: s.removed,
                token: s.token,
            })
        })
    }

    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let (b, id) = (self.0.clone(), id.to_string());
        Box::pin(async move {
            let path = blocking(move || {
                b.string_call(
                    "readOriginal",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    Some(&id),
                )
            })
            .await?;
            Ok(PathBuf::from(path))
        })
    }

    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>> {
        let b = self.0.clone();
        Box::pin(async move {
            let json = blocking(move || {
                Ok(b.call("conditionsJson", "()Ljava/lang/String;", &[])?
                    .unwrap_or_default())
            })
            .await?;
            let c: ConditionsJson = serde_json::from_str(&json)?;
            Ok(Conditions {
                on_wifi: c.on_wifi,
                charging: c.charging,
                battery_pct: c.battery_pct,
            })
        })
    }
}

#[derive(Debug)]
struct JniKeystore(Arc<Backend>);

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Backend")
    }
}

impl SecretStore for JniKeystore {
    fn load(&self, name: &str) -> Result<Option<[u8; 32]>> {
        let hex_text = self.0.string_call(
            "secretLoad",
            "(Ljava/lang/String;)Ljava/lang/String;",
            Some(name),
        )?;
        if hex_text.is_empty() {
            return Ok(None);
        }
        Ok(Some(hex::decode(hex_text)?.try_into().map_err(|_| {
            anyhow!("the stored secret has the wrong length")
        })?))
    }

    fn store(&self, name: &str, secret: &[u8; 32]) -> Result<()> {
        self.0.call(
            "secretStore",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[Some(name), Some(&hex::encode(secret))],
        )?;
        Ok(())
    }

    fn load_or_create(&self, name: &str) -> Result<[u8; 32]> {
        match self.load(name)? {
            Some(s) => Ok(s),
            None => bail!("the device key is missing from the keystore; open the app once"),
        }
    }

    fn describe(&self) -> String {
        "the Android Keystore".into()
    }
}

fn run(
    data_dir: String,
    budget_secs: u64,
    backend: Arc<Backend>,
    last_notified_ms: u64,
) -> Result<serde_json::Value> {
    let dir = PathBuf::from(&data_dir);
    // The app holds `app.lock` while it runs; then its own core does this work.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("app.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Ok(json!({ "skipped": "the app is running" }));
        }
        Err(std::fs::TryLockError::Error(e)) => {
            bail!("cannot lock {}: {e}", dir.join("app.lock").display())
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let store: Arc<dyn SecretStore> = Arc::new(JniKeystore(backend.clone()));
        let core = AppCore::open(
            &dir,
            AppOptions {
                secrets: Some(store),
                ..AppOptions::default()
            },
        )
        .await?;
        if !core.app_state().await?.has_library {
            core.shutdown().await?;
            return Ok(json!({ "skipped": "no library on this device yet" }));
        }
        core.set_photo_source(Arc::new(JniPhotoSource(backend)));
        let backup = if core.backup_status().await?.policy.enabled {
            match core.backup_run(true).await {
                Ok(r) => json!(r),
                Err(e) => json!({ "error": format!("{e:#}") }),
            }
        } else {
            serde_json::Value::Null
        };
        let burst = core.sync_burst(Duration::from_secs(budget_secs)).await?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let notification = decide(NotifyInput {
            at_risk: burst.at_risk_after,
            peer_reachable: burst.peers_reached > 0,
            budget_exhausted: burst.budget_exhausted,
            last_notified_ms: (last_notified_ms > 0).then_some(last_notified_ms),
            now_ms,
            eta_secs: None,
        })
        .map(|n| json!({ "title": n.title, "body": n.body }));
        core.shutdown().await?;
        Ok(json!({ "backup": backup, "burst": burst, "notification": notification }))
    })
}

#[no_mangle]
pub extern "system" fn Java_org_shoebox_desktop_NativeBridge_runBurst(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    budget_secs: jint,
    backend: JObject,
    last_notified_ms: jlong,
) -> jstring {
    let result = (|| -> Result<serde_json::Value> {
        let data_dir: String = env.get_string(&data_dir)?.into();
        let backend = Arc::new(Backend {
            vm: env.get_java_vm()?,
            obj: env.new_global_ref(&backend)?,
        });
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(
                data_dir,
                budget_secs.max(5) as u64,
                backend,
                last_notified_ms.max(0) as u64,
            )
        }))
        .map_err(|_| anyhow!("the background run panicked"))?
    })();
    let text = result
        .unwrap_or_else(|e| json!({ "error": format!("{e:#}") }))
        .to_string();
    env.new_string(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
