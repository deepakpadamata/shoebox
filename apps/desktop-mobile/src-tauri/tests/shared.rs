//! M5: shared albums through the app core: two households, adding with and without location,
//! viewing, saving to the library, the activity feed.

use desktop_mobile::{AppCore, AppOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn sample(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/samples")
        .join(name)
        .display()
        .to_string()
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

fn gps_in(bytes: &[u8]) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let p: PathBuf = dir.path().join("x.jpg");
    std::fs::write(&p, bytes).unwrap();
    shoebox_media::analyze(&p).unwrap().exif.gps.is_some()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn albums_are_shared_without_location_saved_and_followed() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a = open(da.path()).await;
    a.create_library("Ana's laptop", "keeper").await.unwrap();
    let b = open(db.path()).await;
    b.create_library("Ben's laptop", "keeper").await.unwrap();
    a.import_paths(&[sample("photo-gps.jpg"), sample("screenshot.png")])
        .await
        .unwrap();
    let ids: Vec<String> = a
        .library_page(None, 50)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), 2);

    let album = a.shared_create("Lisbon").await.unwrap();
    assert_eq!(
        (album.role.as_str(), album.location.as_str()),
        ("owner", "ask")
    );

    // One of the two photos has a location: the app has to ask before sharing.
    let check = a.shared_add_check(&album.id, &ids).await.unwrap();
    assert_eq!((check.total, check.with_location), (2, 1));
    let err = a
        .shared_add(&album.id, &ids, None, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("location"), "{err}");

    let done = a
        .shared_add(&album.id, &ids, Some("remove".into()), true)
        .await
        .unwrap();
    assert_eq!((done.added, done.location_removed), (2, 1), "{done:?}");
    let listed = a.shared_albums().await.unwrap();
    assert_eq!(
        (listed[0].item_count, listed[0].location.as_str()),
        (2, "remove")
    );

    // Ben joins with the invitation.
    let invite = a.shared_invite(&album.id, "contributor", 7).await.unwrap();
    assert!(invite.qr_svg.contains("<svg"));
    let joined = b.shared_join(&invite.text).await.unwrap();
    assert_eq!(
        (joined.id.as_str(), joined.role.as_str()),
        (album.id.as_str(), "contributor")
    );
    let (b2, id) = (b.clone(), album.id.clone());
    assert!(
        wait(40, || {
            let (b2, id) = (b2.clone(), id.clone());
            async move {
                b2.shared_items(&id, None, 100)
                    .await
                    .map(|p| p.items.len() == 2 && p.items.iter().all(|i| i.has_thumb))
                    .unwrap_or(false)
            }
        })
        .await,
        "Ben sees both photos with thumbnails"
    );
    let page = b.shared_items(&album.id, None, 100).await.unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items.iter().filter(|i| i.location_removed).count(), 1);
    assert!(
        page.items.iter().all(|i| i.added_by == "Ana's laptop"),
        "{:?}",
        page.items
    );

    // What Ben receives has no location, and the GPS original's hash never reaches him.
    let stripped = page.items.iter().find(|i| i.location_removed).unwrap();
    b.shared_fetch(&album.id, &stripped.id, "original")
        .await
        .unwrap();
    let kind = format!("album:{}:original", album.id);
    let bytes = b
        .blob(&stripped.id, &kind, None)
        .await
        .unwrap()
        .expect("served");
    assert!(!gps_in(&bytes.bytes), "no GPS in the shared copy");
    assert!(
        !ids.contains(&stripped.id),
        "the shared item is the stripped copy, not the library's original"
    );
    let thumb = b
        .blob(&stripped.id, &format!("album:{}:thumb", album.id), None)
        .await
        .unwrap();
    assert!(thumb.is_some());

    // Ben saves one into his library; saving twice is harmless.
    assert_eq!(
        b.shared_save(&album.id, std::slice::from_ref(&stripped.id))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        b.shared_save(&album.id, std::slice::from_ref(&stripped.id))
            .await
            .unwrap(),
        1
    );
    let lib = b.library_page(None, 50).await.unwrap();
    assert_eq!(lib.items.len(), 1);
    assert!(lib.items[0].has_original);

    // Ben adds one of his own; Ana sees it and the feed groups the two adds by Ana.
    b.import_paths(&[sample("IMG_0001.jpg")]).await.unwrap();
    let mine: Vec<String> = b
        .library_page(None, 50)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .filter(|id| *id != lib.items[0].id)
        .collect();
    b.shared_add(&album.id, &mine, Some("keep".into()), false)
        .await
        .unwrap();
    let (a2, id) = (a.clone(), album.id.clone());
    assert!(
        wait(40, || {
            let (a2, id) = (a2.clone(), id.clone());
            async move {
                a2.shared_items(&id, None, 10)
                    .await
                    .map(|p| p.total == 3)
                    .unwrap_or(false)
            }
        })
        .await,
        "Ana gets Ben's photo"
    );
    let members = a.shared_members(&album.id).await.unwrap();
    assert!(
        members
            .iter()
            .any(|m| m.name == "Ben's laptop" && m.role == "contributor"),
        "{members:?}"
    );
    let feed = a.shared_feed(&album.id).await.unwrap();
    assert!(
        feed.iter()
            .any(|f| f.kind == "added" && f.member == "Ana's laptop" && f.count == 2),
        "{feed:?}"
    );
    assert!(
        feed.iter()
            .any(|f| f.kind == "joined" && f.member == "Ben's laptop"),
        "{feed:?}"
    );

    // Unread dot for Ana until she opens it.
    let unread = a.shared_albums().await.unwrap()[0].unread;
    assert_eq!(unread, 1);
    a.shared_seen(&album.id).await.unwrap();
    assert_eq!(a.shared_albums().await.unwrap()[0].unread, 0);

    // Viewers cannot add; leaving removes the album from the list.
    let viewer = a.shared_invite(&album.id, "viewer", 7).await.unwrap();
    assert!(viewer.text != invite.text);
    b.shared_leave(&album.id).await.unwrap();
    assert!(b.shared_albums().await.unwrap().is_empty());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keeping_the_location_shares_the_photo_as_it_is() {
    let d = tempfile::tempdir().unwrap();
    let a = open(d.path()).await;
    a.create_library("Ana", "keeper").await.unwrap();
    a.import_paths(&[sample("photo-gps.jpg")]).await.unwrap();
    let ids: Vec<String> = a
        .library_page(None, 5)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();
    let album = a.shared_create("Trip").await.unwrap();
    // "keep" shares as is and says so in the album's settings only when asked to remember.
    let r = a
        .shared_add(&album.id, &ids, Some("keep".into()), false)
        .await
        .unwrap();
    assert_eq!((r.added, r.location_removed), (1, 0));
    assert_eq!(a.shared_albums().await.unwrap()[0].location, "ask");
    a.shutdown().await.unwrap();
}

/// Pair `b` into `a`'s library (both confirm the six digits).
async fn pair(a: &Arc<AppCore>, b: &Arc<AppCore>, name: &str, role: &str) {
    use desktop_mobile::types::AppEvent;
    let start = a.pair_start(role).await.unwrap();
    let confirm = |mut ev: tokio::sync::broadcast::Receiver<AppEvent>, core: Arc<AppCore>| {
        tokio::spawn(async move {
            while let Ok(e) = ev.recv().await {
                if let AppEvent::PairingCode { .. } = e {
                    core.confirm_pairing(true).unwrap();
                    return;
                }
            }
        })
    };
    let (ha, hb) = (
        confirm(a.events(), a.clone()),
        confirm(b.events(), b.clone()),
    );
    b.join(&start.ticket, name).await.unwrap();
    let _ = (ha.await, hb.await);
}

/// A-097: a person's name and shared albums follow them to all their devices; another person
/// sees them once; an album of their own can be shared and stays linked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_albums_follow_the_person_to_all_their_devices() {
    let (d1, d2, db) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let laptop = open(d1.path()).await;
    laptop
        .create_library("Ana's laptop", "keeper")
        .await
        .unwrap();
    let phone = open(d2.path()).await;
    pair(&laptop, &phone, "Ana's phone", "companion").await;
    let ben = open(db.path()).await;
    ben.create_library("Ben's laptop", "keeper").await.unwrap();

    // One name for Ana, set on the laptop, seen on the phone.
    assert!(!laptop.profile().await.unwrap().is_set);
    laptop.set_profile_name("Ana").await.unwrap();
    assert!(
        wait(30, || async {
            phone
                .profile()
                .await
                .map(|p| p.name == "Ana")
                .unwrap_or(false)
        })
        .await,
        "the phone uses the name set on the laptop"
    );

    // Ben invites Ana; she joins on the laptop and the phone takes part by itself.
    let album = ben.shared_create("Picnic").await.unwrap();
    let invite = ben
        .shared_invite(&album.id, "contributor", 7)
        .await
        .unwrap();
    laptop.shared_join(&invite.text).await.unwrap();
    let id = album.id.clone();
    assert!(
        wait(90, || async {
            phone
                .shared_albums()
                .await
                .map(|l| l.iter().any(|a| a.id == id && a.role == "contributor"))
                .unwrap_or(false)
        })
        .await,
        "the phone is in the album too"
    );
    // The phone can add, and Ben sees one Ana with two devices, joined once.
    phone
        .import_paths(&[sample("screenshot.png")])
        .await
        .unwrap();
    let shot = phone.library_page(None, 10).await.unwrap().items[0]
        .id
        .clone();
    phone
        .shared_add(&album.id, &[shot], None, false)
        .await
        .unwrap();
    assert!(
        wait(60, || async {
            ben.shared_members(&id)
                .await
                .map(|m| m.iter().any(|r| r.name == "Ana" && r.devices == 2))
                .unwrap_or(false)
        })
        .await,
        "Ben sees Ana once, with two devices: {:?}",
        ben.shared_members(&id).await
    );
    let members = ben.shared_members(&id).await.unwrap();
    assert_eq!(members.len(), 2, "Ben and Ana: {members:?}");
    let feed = ben.shared_feed(&id).await.unwrap();
    let joins = feed.iter().filter(|f| f.kind == "joined").count();
    assert_eq!(joins, 1, "{feed:?}");

    // Leaving on the phone leaves on the laptop too.
    phone.shared_leave(&id).await.unwrap();
    assert!(
        wait(60, || async {
            laptop
                .shared_albums()
                .await
                .map(|l| !l.iter().any(|a| a.id == id))
                .unwrap_or(false)
        })
        .await,
        "the laptop left as well"
    );

    // Sharing an album Ana already has: its photos go, and what she adds later follows.
    laptop
        .import_paths(&[sample("IMG_0001.jpg"), sample("photo-gps.jpg")])
        .await
        .unwrap();
    let ids: Vec<String> = laptop
        .library_page(None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|p| p.id)
        .collect();
    let trip = laptop.create_album("Trip").await.unwrap();
    laptop.album_add(&trip.id, &ids[..1]).await.unwrap();
    let check = laptop.album_share_check(&trip.id).await.unwrap();
    assert_eq!(check.total, 1);
    let shared = laptop
        .album_share(&trip.id, Some("remove".into()))
        .await
        .unwrap();
    assert_eq!((shared.name.as_str(), shared.item_count), ("Trip", 1));
    assert!(laptop
        .albums()
        .await
        .unwrap()
        .iter()
        .any(|a| a.id == trip.id && a.shared_as.as_deref() == Some(shared.id.as_str())));
    let invite = laptop.shared_invite(&shared.id, "viewer", 7).await.unwrap();
    ben.shared_join(&invite.text).await.unwrap();
    laptop.album_add(&trip.id, &ids[1..]).await.unwrap();
    let (sid, want) = (shared.id.clone(), ids.len() as u32);
    assert!(
        wait(60, || async {
            ben.shared_items(&sid, None, 10)
                .await
                .map(|p| p.total == want)
                .unwrap_or(false)
        })
        .await,
        "the photos added to Trip later reach Ben: {:?}",
        ben.shared_items(&sid, None, 10).await.map(|p| p.total)
    );
    let page = ben.shared_items(&sid, None, 10).await.unwrap();
    assert!(
        page.items.iter().all(|i| i.added_by == "Ana"),
        "{:?}",
        page.items
    );
}
