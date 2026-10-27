//! A person's shared albums across their devices (A-097): the library (replicated to every device
//! of the person) keeps the list of shared albums they are in and the name they show to others.
//! Each device follows that list: it takes part in albums another of its devices created or
//! joined, and leaves the ones the person left.

use crate::shared::{AlbumRef, SharedAlbums};
use anyhow::Result;
use shoebox_core::clock::{Clock, SystemClock};
use shoebox_core::model::AlbumId;
use shoebox_core::store::{LibraryStore, PeerAddr};
use shoebox_node::LibraryNode;
use std::time::Duration;
use tracing::debug;

/// Library setting: JSON list of [`AlbumRef`] (including albums left, with `left_ms`).
pub const ALBUMS_KEY: &str = "shared.albums";
/// Library setting: the person's name as other people see it in shared albums (UTF-8).
pub const NAME_KEY: &str = "profile.name";

const ROUND: Duration = Duration::from_secs(20);

async fn read_list<L: LibraryStore>(store: &L) -> Result<Vec<AlbumRef>> {
    Ok(match store.setting(ALBUMS_KEY).await? {
        Some(b) => serde_json::from_slice(&b).unwrap_or_default(),
        None => vec![],
    })
}

async fn write_list<L: LibraryStore>(store: &L, list: &[AlbumRef]) -> Result<()> {
    store
        .put_setting(ALBUMS_KEY, serde_json::to_vec(list)?)
        .await?;
    Ok(())
}

/// The name the person shows in shared albums, if they set one.
pub async fn person_name<L: LibraryStore>(store: &L) -> Result<Option<String>> {
    Ok(store
        .setting(NAME_KEY)
        .await?
        .and_then(|b| String::from_utf8(b).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

pub async fn set_person_name<L: LibraryStore>(store: &L, name: &str) -> Result<()> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "your name cannot be empty");
    anyhow::ensure!(
        name.chars().count() <= 60,
        "keep your name under 60 characters"
    );
    store
        .put_setting(NAME_KEY, name.as_bytes().to_vec())
        .await?;
    Ok(())
}

/// The shared album a personal album was shared as, if any.
pub async fn shared_from<L: LibraryStore>(
    store: &L,
    personal: &AlbumId,
) -> Result<Option<AlbumId>> {
    Ok(read_list(store)
        .await?
        .into_iter()
        .find(|r| r.from_album == Some(*personal) && r.left_ms.is_none())
        .map(|r| r.album))
}

/// Every personal album that is shared: (personal album, shared album, location choice).
pub async fn shared_links<L: LibraryStore>(
    store: &L,
) -> Result<Vec<(AlbumId, AlbumId, Option<String>)>> {
    Ok(read_list(store)
        .await?
        .into_iter()
        .filter(|r| r.left_ms.is_none())
        .filter_map(|r| r.from_album.map(|p| (p, r.album, r.location)))
        .collect())
}

/// Bring the library's list and this device's albums together once.
pub async fn reconcile<L: LibraryStore + 'static>(
    shared: &SharedAlbums,
    ln: &LibraryNode<L>,
    my_name: &str,
) -> Result<()> {
    let store = ln.store();
    let now = SystemClock.now_ms();
    let mut list = read_list(store.as_ref()).await?;
    let mut changed = false;
    // What this device is in: make sure the library knows (newer capabilities win).
    for local in shared.refs().await {
        match list.iter_mut().find(|r| r.album == local.album) {
            None => {
                list.push(AlbumRef {
                    joined_ms: now,
                    ..local
                });
                changed = true;
            }
            Some(r) if r.left_ms.is_some() => {
                // The person left this album on another device: leave here too.
                debug!(
                    "leaving shared album {}: left on another device",
                    hex::encode(r.album.0)
                );
                shared.leave(&local.album).await?;
            }
            Some(r) if local.cap_ms > r.cap_ms && local.capability != r.capability => {
                r.capability = local.capability;
                r.cap_ms = local.cap_ms;
                r.inviter = local.inviter.or(r.inviter.take());
                changed = true;
            }
            Some(_) => {}
        }
    }
    if changed {
        write_list(store.as_ref(), &list).await?;
    }
    // What the person is in but this device is not (yet): take part.
    let me = ln.device();
    let peers: Vec<PeerAddr> = store
        .device_list()
        .await?
        .devices
        .iter()
        .filter(|d| !d.revoked && d.device != me)
        .map(|d| PeerAddr::id_only(d.device))
        .collect();
    for r in list.iter().filter(|r| r.left_ms.is_none()) {
        if let Err(e) = shared.adopt(r, peers.clone(), my_name).await {
            debug!(
                "could not take part in shared album {} yet: {e:#}",
                hex::encode(r.album.0)
            );
        }
    }
    Ok(())
}

/// Leave a shared album on every device of the person.
pub async fn leave_everywhere<L: LibraryStore + 'static>(
    shared: &SharedAlbums,
    ln: &LibraryNode<L>,
    album: &AlbumId,
) -> Result<()> {
    let store = ln.store();
    let mut list = read_list(store.as_ref()).await?;
    let now = SystemClock.now_ms();
    match list.iter_mut().find(|r| r.album == *album) {
        Some(r) => r.left_ms = Some(now),
        None => {
            if let Some(local) = shared.refs().await.into_iter().find(|r| r.album == *album) {
                list.push(AlbumRef {
                    joined_ms: now,
                    left_ms: Some(now),
                    ..local
                });
            }
        }
    }
    write_list(store.as_ref(), &list).await?;
    shared.leave(album).await
}

/// Record that `shared_album` was shared from the personal album `personal`, with the location
/// choice every device uses when it adds photos later.
pub async fn link<L: LibraryStore + 'static>(
    shared: &SharedAlbums,
    ln: &LibraryNode<L>,
    shared_album: &AlbumId,
    personal: &AlbumId,
    location: Option<String>,
) -> Result<()> {
    let store = ln.store();
    let mut list = read_list(store.as_ref()).await?;
    if !list.iter().any(|r| r.album == *shared_album) {
        if let Some(local) = shared
            .refs()
            .await
            .into_iter()
            .find(|r| r.album == *shared_album)
        {
            list.push(AlbumRef {
                joined_ms: SystemClock.now_ms(),
                ..local
            });
        }
    }
    for r in list.iter_mut().filter(|r| r.album == *shared_album) {
        r.from_album = Some(*personal);
        r.location = location.clone();
    }
    write_list(store.as_ref(), &list).await
}

/// Keep following the library: on every library change and every 20 s. `name` gives the name
/// to show when the person set none (it can change). Abort the handle when the library closes.
pub fn follow<L: LibraryStore + 'static>(
    shared: SharedAlbums,
    ln: LibraryNode<L>,
    name: impl Fn() -> String + Send + Sync + 'static,
) -> tokio::task::JoinHandle<()> {
    shared.set_person(ln.person_id());
    tokio::spawn(async move {
        let mut changes = ln.store().subscribe();
        loop {
            let my_name = match person_name(ln.store().as_ref()).await {
                Ok(Some(n)) => n,
                _ => name(),
            };
            if let Err(e) = reconcile(&shared, &ln, &my_name).await {
                debug!("shared albums: following the library failed: {e:#}");
            }
            let _ = tokio::time::timeout(ROUND, futures::StreamExt::next(&mut changes)).await;
            // Let a burst of library changes settle before looking again.
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    })
}
