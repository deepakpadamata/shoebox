//! M3-01/03/04: the app core end to end (no UI): creating a library, the timeline, photo detail,
//! edits, albums, devices, health, settings and pairing between two app instances.

use desktop_mobile::types::*;
use desktop_mobile::{AppCore, AppOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/samples")
}

fn sample(name: &str) -> String {
    samples().join(name).display().to_string()
}

async fn open(dir: &Path) -> Arc<AppCore> {
    AppCore::open(dir, AppOptions::local_only()).await.unwrap()
}

async fn wait<F: FnMut() -> Fut, Fut: std::future::Future<Output = bool>>(
    secs: u64,
    mut f: F,
) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

async fn library_with_photos(dir: &Path) -> Arc<AppCore> {
    let core = open(dir).await;
    core.create_library("MacBook", "keeper").await.unwrap();
    let report = core
        .import_paths(&[
            sample("photo-gps.jpg"),
            sample("IMG_0001.jpg"),
            sample("screenshot.png"),
        ])
        .await
        .unwrap();
    assert_eq!(report.imported, 3, "{report:?}");
    core
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_device_starts_empty_and_creating_a_library_hands_out_the_kit_once() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path()).await;
    let st = core.app_state().await.unwrap();
    assert!(!st.has_library && st.library_id.is_none());
    assert!(core.library_page(None, 50).await.is_err(), "no library yet");

    let created = core.create_library("MacBook", "keeper").await.unwrap();
    assert_eq!(created.phrase.split_whitespace().count(), 24);
    assert!(created.kit_text.contains(&created.library_id));
    assert!(
        core.create_library("again", "keeper").await.is_err(),
        "only one library per device"
    );

    let st = core.app_state().await.unwrap();
    assert!(st.has_library && st.membership == "member" && !st.recovery_kit_saved);
    assert_eq!(st.device_name, "MacBook");
    core.mark_recovery_kit_saved().await.unwrap();
    assert!(core.app_state().await.unwrap().recovery_kit_saved);
    core.shutdown().await.unwrap();

    // A restart reopens the same library without asking again.
    let again = open(dir.path()).await;
    let st2 = again.app_state().await.unwrap();
    assert!(st2.has_library && st2.library_id == st.library_id && st2.device_id == st.device_id);
    again.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_timeline_pages_newest_first_and_details_say_where_a_photo_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let core = library_with_photos(dir.path()).await;

    let page = core.library_page(None, 2).await.unwrap();
    assert_eq!(page.items.len(), 2);
    let next = page.next.clone().expect("a second page exists");
    let rest = core.library_page(Some(next), 2).await.unwrap();
    assert_eq!(rest.items.len(), 1);
    assert!(rest.next.is_none());
    let all: Vec<&PhotoSummary> = page.items.iter().chain(rest.items.iter()).collect();
    assert!(
        all.windows(2).all(|w| w[0].taken_at >= w[1].taken_at),
        "newest first"
    );
    assert!(all
        .iter()
        .all(|p| p.has_thumb && p.has_original && p.copies == 1));

    let id = &all[0].id;
    let d = core.photo_detail(id).await.unwrap();
    assert_eq!(d.copies, 1);
    assert_eq!(d.redundancy_target, 2);
    assert!(d.has_original && !d.pinned && !d.deleted);
    assert_eq!(d.stored_on.len(), 1);
    assert!(
        d.stored_on[0].is_this_device && d.stored_on[0].kinds.contains(&"original".to_string())
    );

    let counts = core.counts().await.unwrap();
    assert_eq!(
        (counts.photos, counts.at_risk, counts.deleted),
        (3, 0, 0),
        "one copy is below the target of 2, but this device is a keeper and keepers count as backed up"
    );
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn favorites_deleting_restoring_pins_albums_and_settings_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let core = library_with_photos(dir.path()).await;
    let ids: Vec<String> = core
        .library_page(None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();

    core.set_favorite(&ids[..1], true).await.unwrap();
    assert!(core.photo_detail(&ids[0]).await.unwrap().favorite);
    assert_eq!(core.counts().await.unwrap().favorites, 1);

    core.delete(&ids[1..2]).await.unwrap();
    assert_eq!(core.library_page(None, 10).await.unwrap().items.len(), 2);
    let bin = core.recently_deleted().await.unwrap();
    assert_eq!(bin.len(), 1);
    assert_eq!(bin[0].id, ids[1]);
    core.restore(&ids[1..2]).await.unwrap();
    assert_eq!(core.library_page(None, 10).await.unwrap().items.len(), 3);

    core.pin_original(&ids[0], true).await.unwrap();
    assert!(core.photo_detail(&ids[0]).await.unwrap().pinned);
    core.pin_original(&ids[0], false).await.unwrap();
    assert!(!core.photo_detail(&ids[0]).await.unwrap().pinned);

    let album = core.create_album("  Summer ").await.unwrap();
    assert_eq!(album.name, "Summer");
    assert!(core.create_album("   ").await.is_err());
    core.album_add(&album.id, &ids[..2]).await.unwrap();
    let page = core.album_page(&album.id, None, 10).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(core.albums().await.unwrap()[0].item_count, 2);
    assert_eq!(
        core.photo_detail(&ids[0]).await.unwrap().albums,
        vec![album.id.clone()]
    );
    core.album_remove(&album.id, &ids[..1]).await.unwrap();
    assert_eq!(
        core.album_page(&album.id, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        1
    );

    core.set_redundancy_target(1).await.unwrap();
    assert!(core.set_redundancy_target(0).await.is_err());
    assert_eq!(core.settings().await.unwrap().redundancy_target, 1);
    assert_eq!(
        core.counts().await.unwrap().at_risk,
        0,
        "one copy meets a target of 1"
    );
    core.rename_device("Studio Mac").await.unwrap();
    assert_eq!(core.settings().await.unwrap().device_name, "Studio Mac");
    assert!(core.set_relay("http://plain.example").await.is_err());
    core.set_relay("off").await.unwrap();
    assert_eq!(core.settings().await.unwrap().relay, "off");

    let h = core.health().await.unwrap();
    assert_eq!(h.photos, 3);
    assert_eq!(h.originals_here, 3);
    assert_eq!(h.devices_total, 1);
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blobs_are_served_whole_in_ranges_and_missing_ones_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let core = library_with_photos(dir.path()).await;
    let id = core.library_page(None, 1).await.unwrap().items[0]
        .id
        .clone();

    let thumb = core
        .blob(&id, "thumb", None)
        .await
        .unwrap()
        .expect("thumbnail is local");
    assert_eq!(thumb.bytes.len() as u64, thumb.total);
    assert!(thumb.range.is_none());
    assert!(thumb.content_type.starts_with("image/"));
    assert_eq!(thumb.etag.len(), 64);
    assert!(
        thumb.bytes.starts_with(&[0xFF, 0xD8])
            || thumb.bytes.starts_with(&[0x89, b'P'])
            || thumb.content_type == "image/webp"
    );

    let part = core
        .blob(&id, "original", Some((10, Some(19))))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(part.bytes.len(), 10);
    assert_eq!(part.range, Some((10, 19)));
    let whole = core.blob(&id, "original", None).await.unwrap().unwrap();
    assert_eq!(
        &whole.bytes[10..20],
        &part.bytes[..],
        "a range is a slice of the whole"
    );
    let tail = core
        .blob(&id, "original", Some((whole.total - 5, None)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tail.bytes.len(), 5);
    let beyond = core
        .blob(&id, "original", Some((whole.total + 100, None)))
        .await
        .unwrap()
        .unwrap();
    assert!(beyond.bytes.is_empty());

    assert!(core.blob("zz", "thumb", None).await.is_err());
    assert!(
        core.blob(&"0".repeat(64), "thumb", None).await.is_err(),
        "unknown photo"
    );
    assert!(core.blob(&id, "huge", None).await.is_err());
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_app_joins_by_pairing_and_the_library_follows() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = library_with_photos(da.path()).await;
    let b = open(db.path()).await;

    let start = a.pair_start("keeper").await.unwrap();
    assert!(start.ticket.starts_with("shoebox-pair1"));
    assert!(start.qr_svg.contains("<svg"));

    // Both apps get the same six digits; both people confirm.
    let (mut ea, mut eb) = (a.events(), b.events());
    let a2 = a.clone();
    let b2 = b.clone();
    let confirm = |mut ev: tokio::sync::broadcast::Receiver<AppEvent>, core: Arc<AppCore>| {
        tokio::spawn(async move {
            while let Ok(e) = ev.recv().await {
                if let AppEvent::PairingCode { sas, .. } = e {
                    core.confirm_pairing(true).unwrap();
                    return Some(sas);
                }
            }
            None
        })
    };
    let (ha, hb) = (confirm(ea.resubscribe(), a2), confirm(eb.resubscribe(), b2));
    b.join(&start.ticket, "Pi").await.unwrap();
    let (sa, sb) = (ha.await.unwrap().unwrap(), hb.await.unwrap().unwrap());
    assert_eq!(sa, sb);
    assert_eq!(sa.len(), 6);
    let _ = (&mut ea, &mut eb);

    let st = b.app_state().await.unwrap();
    assert!(st.has_library && st.role == "keeper" && st.device_name == "Pi");
    assert!(
        wait(30, || async {
            b.library_page(None, 10)
                .await
                .map(|p| p.items.len() == 3)
                .unwrap_or(false)
        })
        .await,
        "the new device syncs the timeline"
    );
    let devices = a.devices().await.unwrap();
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().any(|d| d.name == "Pi" && d.role == "keeper"));
    assert!(devices[0].is_this_device, "this device comes first");

    // The keeper fetches the originals; the first device then sees two copies.
    assert!(
        wait(40, || async {
            a.photo_detail(&a.library_page(None, 1).await.unwrap().items[0].id)
                .await
                .map(|d| d.copies == 2)
                .unwrap_or(false)
        })
        .await,
        "copy count follows"
    );
    // A pairing ticket cannot be reused and an unknown ticket is rejected with a clear message.
    let c = tempfile::tempdir().unwrap();
    let third = open(c.path()).await;
    assert!(third.join(&start.ticket, "Third").await.is_err());
    assert!(third.join("nonsense", "Third").await.is_err());
    for core in [a, b, third] {
        core.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declining_the_code_leaves_the_joining_app_without_a_library() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = library_with_photos(da.path()).await;
    let b = open(db.path()).await;
    let start = a.pair_start("phone").await.unwrap();
    let mut eb = b.events();
    let b2 = b.clone();
    let decline = tokio::spawn(async move {
        while let Ok(e) = eb.recv().await {
            if matches!(e, AppEvent::PairingCode { .. }) {
                b2.confirm_pairing(false).unwrap();
                return;
            }
        }
    });
    let err = b.join(&start.ticket, "Phone").await.unwrap_err();
    decline.await.unwrap();
    assert!(
        format!("{err:#}").to_lowercase().contains("declined"),
        "{err:#}"
    );
    assert!(!b.app_state().await.unwrap().has_library);
    assert_eq!(a.devices().await.unwrap().len(), 1, "nothing was certified");
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exporting_writes_verified_originals_and_a_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let core = library_with_photos(dir.path()).await;
    let ids: Vec<String> = core
        .library_page(None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();
    let album = core.create_album("Trip").await.unwrap();
    core.album_add(&album.id, &ids[..1]).await.unwrap();
    core.set_favorite(&ids[1..2], true).await.unwrap();

    let out = tempfile::tempdir().unwrap();
    let report = core
        .export_library(&out.path().join("backup").display().to_string())
        .await
        .unwrap();
    assert_eq!((report.written, report.missing), (3, 0));
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.path().join("backup/library.json")).unwrap())
            .unwrap();
    let entries = manifest["photos"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let mut total = 0u64;
    for e in entries {
        let file = out.path().join("backup").join(e["file"].as_str().unwrap());
        let data = std::fs::read(&file).unwrap();
        total += data.len() as u64;
        // Content-addressed: the file's hash is the photo id.
        assert_eq!(
            shoebox_core::Blake3Hash::of_bytes(&data).to_string(),
            e["id"].as_str().unwrap()
        );
        assert!(
            file.parent()
                .unwrap()
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .len()
                == 4,
            "year folder"
        );
    }
    assert_eq!(report.bytes as u64, total);
    assert!(entries.iter().any(|e| e["favorite"] == true));
    assert!(entries
        .iter()
        .any(|e| e["albums"] == serde_json::json!(["Trip"])));
    // Running it again changes nothing and still succeeds.
    assert_eq!(
        core.export_library(&report.folder).await.unwrap().written,
        3
    );
    assert!(core.export_library("").await.is_err());
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn albums_can_be_renamed() {
    let dir = tempfile::tempdir().unwrap();
    let core = library_with_photos(dir.path()).await;
    let ids: Vec<String> = core
        .library_page(None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();
    let a = core.create_album("Trip").await.unwrap();
    core.album_add(&a.id, &ids[..2]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    core.rename_album(&a.id, "  Lisbon 2025 ").await.unwrap();
    let albums = core.albums().await.unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(
        (albums[0].name.as_str(), albums[0].item_count),
        ("Lisbon 2025", 2),
        "items survive a rename"
    );
    assert!(core.rename_album(&a.id, "  ").await.is_err());
    assert!(
        core.rename_album(&"0".repeat(32), "x").await.is_err(),
        "unknown album"
    );
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn videos_get_device_local_poster_frames() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path()).await;
    core.create_library("Mac", "keeper").await.unwrap();
    core.import_paths(&[sample("clip.mp4"), sample("photo-gps.jpg")])
        .await
        .unwrap();
    let items = core.library_page(None, 10).await.unwrap().items;
    let video = items
        .iter()
        .find(|p| p.is_video)
        .expect("the clip imported as a video");
    let photo = items.iter().find(|p| !p.is_video).unwrap();
    assert!(!video.has_poster);
    assert!(
        core.blob(&video.id, "poster", None)
            .await
            .unwrap()
            .is_none(),
        "no poster yet"
    );

    let jpeg = [0xFFu8, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
    core.set_poster(&video.id, &b64).await.unwrap();
    let got = core
        .blob(&video.id, "poster", None)
        .await
        .unwrap()
        .expect("poster is served");
    assert_eq!(
        (got.bytes.as_slice(), got.content_type.as_str()),
        (&jpeg[..], "image/jpeg")
    );
    assert!(
        core.library_page(None, 10)
            .await
            .unwrap()
            .items
            .iter()
            .find(|p| p.id == video.id)
            .unwrap()
            .has_poster
    );

    assert!(
        core.set_poster(&photo.id, &b64).await.is_err(),
        "photos do not need posters"
    );
    assert!(
        core.set_poster(&video.id, "AAAA").await.is_err(),
        "not a JPEG"
    );
    assert!(
        core.set_poster(&video.id, "###").await.is_err(),
        "not base64"
    );
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_app_adopts_a_library_made_by_the_dev_tool() {
    // Build what `shoebox-dev init` leaves behind: the node under `node/` plus dev.json.
    let dir = tempfile::tempdir().unwrap();
    let first = open(dir.path()).await;
    let created = first.create_library("Dev laptop", "keeper").await.unwrap();
    first
        .import_paths(&[sample("photo-gps.jpg")])
        .await
        .unwrap();
    let id = first.app_state().await.unwrap().device_id;
    first.shutdown().await.unwrap();
    let app: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("app.json")).unwrap()).unwrap();
    std::fs::remove_file(dir.path().join("app.json")).unwrap();
    std::fs::write(
        dir.path().join("dev.json"),
        serde_json::json!({"name":"Dev laptop","role":"Keeper","ticket":app["ticket"],"peers":[],"root_seed":null}).to_string(),
    )
    .unwrap();

    let adopted = open(dir.path()).await;
    let st = adopted.app_state().await.unwrap();
    assert!(st.has_library);
    assert_eq!(st.device_id, id, "same device key");
    assert_eq!(st.library_id.as_deref(), Some(created.library_id.as_str()));
    assert_eq!(st.device_name, "Dev laptop");
    assert_eq!(
        adopted.library_page(None, 10).await.unwrap().items.len(),
        1,
        "the photos are there"
    );
    assert!(
        dir.path().join("app.json").exists(),
        "remembered for next time"
    );
    adopted.shutdown().await.unwrap();

    // A corrupt dev.json is an error, not a silently empty app.
    let bad = tempfile::tempdir().unwrap();
    std::fs::write(
        bad.path().join("dev.json"),
        b"{\"name\":\"x\",\"role\":\"Keeper\",\"ticket\":\"nope\"}",
    )
    .unwrap();
    assert!(AppCore::open(bad.path(), AppOptions::local_only())
        .await
        .is_err());
}

/// Rejoining with the recovery phrase through this app waits for its person: the question shows
/// in the app state, "Allow" lets the device in (and raises no alert, it was approved here),
/// "Refuse" leaves the other app without a library.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recovery_through_this_app_asks_first() {
    let (da, db, dc) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let a = open(da.path()).await;
    let created = a.create_library("MacBook", "keeper").await.unwrap();
    let link = a.device_link();

    let b = open(db.path()).await;
    let (b2, link2, phrase2) = (b.clone(), link.clone(), created.phrase.clone());
    let joining = tokio::spawn(async move { b2.recover(&link2, &phrase2, "Laptop").await });
    assert!(
        wait(20, || async {
            a.app_state().await.unwrap().recovery_request.is_some()
        })
        .await,
        "the question reaches the app"
    );
    let q = a.app_state().await.unwrap().recovery_request.unwrap();
    assert_eq!(
        (q.device_name.as_str(), q.role.as_str()),
        ("Laptop", "keeper")
    );
    a.answer_recovery(true).unwrap();
    joining.await.unwrap().unwrap();
    assert!(b.app_state().await.unwrap().has_library);
    let st = a.app_state().await.unwrap();
    assert!(st.recovery_request.is_none());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        a.app_state().await.unwrap().alerts.is_empty(),
        "no alert for a device let in here"
    );

    let c = open(dc.path()).await;
    let (c2, link3, phrase3) = (c.clone(), link.clone(), created.phrase.clone());
    let refused = tokio::spawn(async move { c2.recover(&link3, &phrase3, "Unknown").await });
    assert!(
        wait(20, || async {
            a.app_state().await.unwrap().recovery_request.is_some()
        })
        .await
    );
    a.answer_recovery(false).unwrap();
    let err = refused.await.unwrap().unwrap_err();
    assert!(format!("{err:#}").contains("not approved"), "{err:#}");
    assert!(!c.app_state().await.unwrap().has_library);
    assert!(
        a.answer_recovery(true).is_err(),
        "nothing is waiting any more"
    );
}

/// Replacing the recovery phrase hands out a new kit once, asks for it to be saved again, and the
/// library opens under the new root after a restart with every photo; the old phrase opens
/// nothing (a second replacement with it fails).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_recovery_phrase_can_be_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path()).await;
    let created = core.create_library("MacBook", "keeper").await.unwrap();
    core.import_paths(&[sample("photo-gps.jpg"), sample("IMG_0001.jpg")])
        .await
        .unwrap();
    core.mark_recovery_kit_saved().await.unwrap();
    let before = core.app_state().await.unwrap().library_id;

    assert!(core
        .replace_recovery_phrase("not the phrase")
        .await
        .is_err());
    let new = core.replace_recovery_phrase(&created.phrase).await.unwrap();
    assert_ne!(new.phrase, created.phrase);
    assert_eq!(new.phrase.split(' ').count(), 24);
    let st = core.app_state().await.unwrap();
    assert!(!st.recovery_kit_saved, "the new kit has to be saved again");
    assert_ne!(st.library_id, before, "a new epoch");
    assert!(
        core.replace_recovery_phrase(&created.phrase).await.is_err(),
        "the old phrase opens nothing"
    );
    core.shutdown().await.unwrap();

    let again = open(dir.path()).await;
    let st = again.app_state().await.unwrap();
    assert!(st.has_library && st.membership == "member", "{st:?}");
    assert_eq!(again.counts().await.unwrap().photos, 2);
    assert!(again.replace_recovery_phrase(&new.phrase).await.is_ok());
    again.shutdown().await.unwrap();
}

/// R1-05: what Settings → About shows, and the licence notices that ship with the app.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn about_names_the_version_the_source_and_the_licences() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path()).await;
    let about = core.about();
    assert_eq!(about.version, env!("CARGO_PKG_VERSION"));
    assert!(!about.commit.is_empty());
    assert!(about.source_url.starts_with("https://"));
    let text = core.third_party_licences();
    assert!(text.contains("AGPL-3.0-only"));
    assert!(text.contains("\niroh "), "the iroh crate is listed");
    assert!(text.contains("[standard MIT]"));
    core.shutdown().await.unwrap();
}

/// R1-06: a security alert raised here (the phrase was replaced on another app) is handed out
/// once for a system notification (Android's worker, the desktop app with its window hidden), and
/// stays in the app state as a banner until dismissed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn security_alerts_are_notified_once() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = open(da.path()).await;
    let created = a.create_library("MacBook", "keeper").await.unwrap();
    let b = open(db.path()).await;
    let (b2, link, phrase) = (b.clone(), a.device_link(), created.phrase.clone());
    let joining = tokio::spawn(async move { b2.recover(&link, &phrase, "Laptop").await });
    assert!(
        wait(20, || async {
            a.app_state().await.unwrap().recovery_request.is_some()
        })
        .await
    );
    a.answer_recovery(true).unwrap();
    joining.await.unwrap().unwrap();
    assert!(a.take_unnotified_alerts().is_empty(), "nothing yet");
    assert!(
        wait(20, || async {
            b.app_state().await.unwrap().membership == "member"
        })
        .await,
        "the recovered app has synced its own certificate"
    );

    b.replace_recovery_phrase(&created.phrase).await.unwrap();
    assert!(
        wait(20, || async {
            a.app_state()
                .await
                .unwrap()
                .alerts
                .iter()
                .any(|x| x.kind == "phrase_replaced")
        })
        .await,
        "the other app is told the phrase was replaced"
    );
    let first = a.take_unnotified_alerts();
    assert!(
        first.iter().any(|x| x.kind == "phrase_replaced"),
        "{first:?}"
    );
    assert!(a.take_unnotified_alerts().is_empty(), "once");
    assert!(
        a.app_state()
            .await
            .unwrap()
            .alerts
            .iter()
            .all(|x| x.notified),
        "still shown as a banner, marked notified"
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

/// R1-07: the app reads a release feed (here a local one) and shows a newer version until the
/// banner is dismissed; turning the checks off clears it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_newer_release_on_the_feed_shows_until_dismissed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let feed = format!(
        "http://{}/downloads/release.json",
        listener.local_addr().unwrap()
    );
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let body = r#"{"version":"99.1.0","files":[]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(
        dir.path(),
        AppOptions {
            update_feed: feed.clone(),
            ..AppOptions::local_only()
        },
    )
    .await
    .unwrap();
    assert!(
        wait(10, || async {
            core.app_state()
                .await
                .unwrap()
                .update
                .is_some_and(|u| u.version == "99.1.0" && u.page.ends_with("/downloads/"))
        })
        .await,
        "checked at start"
    );
    core.dismiss_update().unwrap();
    assert!(
        core.app_state().await.unwrap().update.is_none(),
        "dismissed"
    );
    assert!(core.settings().await.unwrap().update_checks);
    core.set_update_checks(false).unwrap();
    assert!(!core.settings().await.unwrap().update_checks);
    core.shutdown().await.unwrap();
}
