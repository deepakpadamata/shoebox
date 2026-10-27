//! Fetch policy (M1-12): which blobs a device should have, as a pure function of its role.
//!
//! - **Keeper**: thumbnails, previews and every original.
//! - **Companion**: thumbnails and previews of everything; the original when the user pinned it,
//!   when it is recent (within the device's window), when it is a favourite (if the device keeps
//!   favourites), or when it is this device's own capture that does not yet have enough copies.
//!   Everything else lives on keepers.
//! - **Viewer**: thumbnails and previews only.
//! - **Shared albums**: previews only, plus originals when the album's "keep originals" is on.
//!
//! The caller turns these plans into a queue ordered thumbnails → previews → originals, newest
//! photos first ([`queue_order`]).

use crate::ids::Timestamp;
use crate::model::{BlobKind, BlobRef, PhotoId, PhotoRecord, Role, SharedItem};

/// Facts about one photo that decide whether this device wants its original.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanContext {
    /// The user pinned this original on this device.
    pub pinned: bool,
    /// This device imported the photo and fewer than the target number of copies exist.
    pub own_capture_below_target: bool,
    /// The photo was taken within this device's keep-recent window.
    pub recent: bool,
    /// The photo is a favourite and this device keeps favourites.
    pub kept_favorite: bool,
}

impl PlanContext {
    /// A Companion holds this original on its own account (not just because it is pinned).
    pub fn keeps_original(&self) -> bool {
        self.pinned || self.own_capture_below_target || self.recent || self.kept_favorite
    }
}

/// One blob to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchItem {
    pub kind: BlobKind,
    pub blob: BlobRef,
}

fn push_unique(plan: &mut Vec<FetchItem>, kind: BlobKind, blob: &BlobRef) {
    // Videos use the original as their own thumbnail/preview until poster frames exist (M3-10):
    // that blob must be treated as an original, never fetched under a smaller kind.
    if plan.iter().all(|i| i.blob.hash != blob.hash) {
        plan.push(FetchItem {
            kind,
            blob: blob.clone(),
        });
    }
}

fn is_placeholder(rec_original: &BlobRef, rendition: &BlobRef) -> bool {
    rec_original.hash == rendition.hash
}

/// Blobs of a library photo this device should hold, in priority order
/// (thumbnail, preview, original).
pub fn fetch_plan(role: Role, rec: &PhotoRecord, ctx: &PlanContext) -> Vec<FetchItem> {
    let want_original = match role {
        Role::Keeper => true,
        Role::Companion => ctx.keeps_original(),
        Role::Viewer => false,
    };
    let mut plan = vec![];
    if !is_placeholder(&rec.original, &rec.thumb) {
        push_unique(&mut plan, BlobKind::Thumb, &rec.thumb);
    }
    if !is_placeholder(&rec.original, &rec.preview) {
        push_unique(&mut plan, BlobKind::Preview, &rec.preview);
    }
    if want_original {
        push_unique(&mut plan, BlobKind::Original, &rec.original);
    }
    plan
}

/// Blobs of a shared-album item this device should hold: always the thumbnail and preview, and
/// the original only for Keepers whose album has "keep originals" on (F6.9). The stripped copy
/// (`shared_hash`, location removed) is what members fetch when it exists.
pub fn shared_item_plan(role: Role, item: &SharedItem, keep_originals: bool) -> Vec<FetchItem> {
    let mut plan = vec![];
    if !is_placeholder(&item.original, &item.thumb) {
        push_unique(&mut plan, BlobKind::Thumb, &item.thumb);
    }
    if !is_placeholder(&item.original, &item.preview) {
        push_unique(&mut plan, BlobKind::Preview, &item.preview);
    }
    if role == Role::Keeper && keep_originals {
        let blob = match item.shared_hash {
            Some(hash) => BlobRef {
                hash,
                size: item.original.size,
                mime: item.original.mime.clone(),
            },
            None => item.original.clone(),
        };
        push_unique(&mut plan, BlobKind::Original, &blob);
    }
    plan
}

/// A planned fetch with what is needed to order it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFetch {
    pub photo: PhotoId,
    pub taken_at: Timestamp,
    pub item: FetchItem,
}

fn kind_rank(kind: BlobKind) -> u8 {
    match kind {
        BlobKind::Thumb => 0,
        BlobKind::Preview => 1,
        BlobKind::Original => 2,
    }
}

/// Order a queue: all thumbnails before any preview before any original; within a kind, newest
/// photo first (ties by photo id, then blob hash, so the order is deterministic).
pub fn queue_order(queue: &mut [QueuedFetch]) {
    queue.sort_by(|a, b| {
        (
            kind_rank(a.item.kind),
            std::cmp::Reverse(a.taken_at),
            a.photo,
            a.item.blob.hash,
        )
            .cmp(&(
                kind_rank(b.item.kind),
                std::cmp::Reverse(b.taken_at),
                b.photo,
                b.item.blob.hash,
            ))
    });
}

/// Preference among devices that hold a blob (best first): a better network path beats a worse
/// one (LAN > direct > relay > unknown), then the most recently seen device, then device id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathQuality {
    Lan = 0,
    Direct = 1,
    Relay = 2,
    Unknown = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate<D> {
    pub device: D,
    pub path: PathQuality,
    /// Milliseconds since the device was last seen (smaller = fresher); `None` = never.
    pub last_seen_ago_ms: Option<u64>,
}

pub fn rank_holders<D: Ord + Clone>(candidates: &mut [Candidate<D>]) {
    candidates.sort_by(|a, b| {
        let seen = |c: &Candidate<D>| c.last_seen_ago_ms.unwrap_or(u64::MAX);
        (a.path, seen(a), &a.device).cmp(&(b.path, seen(b), &b.device))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Hlc;
    use crate::ids::Blake3Hash;
    use crate::ids::EndpointId;
    use crate::merge::Lww;
    use crate::model::ExifSummary;

    fn blob(n: u8) -> BlobRef {
        BlobRef {
            hash: Blake3Hash([n; 32]),
            size: 1,
            mime: "image/jpeg".into(),
        }
    }

    fn photo(n: u8, taken: i64) -> PhotoRecord {
        PhotoRecord {
            id: Blake3Hash([n; 32]),
            original: blob(n),
            preview: blob(n + 100),
            thumb: blob(n + 200),
            taken_at: Timestamp(taken),
            exif: ExifSummary::default(),
            imported_by: EndpointId([1; 32]),
            favorite: Lww::new(false, Hlc::GENESIS),
            deleted_at: Lww::new(None, Hlc::GENESIS),
        }
    }

    fn kinds(plan: &[FetchItem]) -> Vec<BlobKind> {
        plan.iter().map(|i| i.kind).collect()
    }

    #[test]
    fn keeper_fetches_everything_in_priority_order() {
        let plan = fetch_plan(Role::Keeper, &photo(1, 0), &PlanContext::default());
        assert_eq!(
            kinds(&plan),
            vec![BlobKind::Thumb, BlobKind::Preview, BlobKind::Original]
        );
    }

    #[test]
    fn viewer_never_plans_an_original_whatever_the_context() {
        for pinned in [false, true] {
            for own in [false, true] {
                let ctx = PlanContext {
                    pinned,
                    own_capture_below_target: own,
                    recent: true,
                    kept_favorite: true,
                };
                let plan = fetch_plan(Role::Viewer, &photo(1, 0), &ctx);
                assert_eq!(kinds(&plan), vec![BlobKind::Thumb, BlobKind::Preview]);
            }
        }
    }

    #[test]
    fn a_companion_wants_an_original_when_pinned_recent_favourite_or_an_under_replicated_own_capture(
    ) {
        let p = photo(1, 0);
        let plan = |ctx: PlanContext| fetch_plan(Role::Companion, &p, &ctx);
        let none = PlanContext::default();
        assert_eq!(kinds(&plan(none)), vec![BlobKind::Thumb, BlobKind::Preview]);
        for ctx in [
            PlanContext {
                pinned: true,
                ..none
            },
            PlanContext {
                own_capture_below_target: true,
                ..none
            },
            PlanContext {
                recent: true,
                ..none
            },
            PlanContext {
                kept_favorite: true,
                ..none
            },
        ] {
            assert!(kinds(&plan(ctx)).contains(&BlobKind::Original), "{ctx:?}");
        }
    }

    #[test]
    fn a_video_placeholder_is_only_ever_fetched_as_the_original() {
        let mut v = photo(1, 0);
        v.preview = v.original.clone();
        v.thumb = v.original.clone();
        assert!(
            fetch_plan(Role::Viewer, &v, &PlanContext::default()).is_empty(),
            "a viewer does not pull whole videos"
        );
        let keeper = fetch_plan(Role::Keeper, &v, &PlanContext::default());
        assert_eq!(keeper.len(), 1);
        assert_eq!(keeper[0].kind, BlobKind::Original);
    }

    #[test]
    fn shared_albums_give_previews_and_originals_only_to_keepers_with_keep_originals() {
        let item = SharedItem {
            id: Blake3Hash([1; 32]),
            original: blob(1),
            shared_hash: None,
            preview: blob(2),
            thumb: blob(3),
            added_by: EndpointId([1; 32]),
            added_at: Timestamp(0),
            taken_at: Timestamp(0),
        };
        for role in [Role::Companion, Role::Viewer] {
            assert_eq!(
                kinds(&shared_item_plan(role, &item, true)),
                vec![BlobKind::Thumb, BlobKind::Preview]
            );
        }
        assert_eq!(
            kinds(&shared_item_plan(Role::Keeper, &item, false)),
            vec![BlobKind::Thumb, BlobKind::Preview]
        );
        assert!(kinds(&shared_item_plan(Role::Keeper, &item, true)).contains(&BlobKind::Original));
        // A location-stripped copy replaces the original as the thing to keep.
        let mut stripped = item.clone();
        stripped.shared_hash = Some(Blake3Hash([9; 32]));
        let plan = shared_item_plan(Role::Keeper, &stripped, true);
        assert_eq!(plan.last().unwrap().blob.hash, Blake3Hash([9; 32]));
    }

    #[test]
    fn queue_puts_thumbs_before_previews_before_originals_newest_first() {
        let mut queue = vec![];
        for (n, taken) in [(1u8, 100i64), (2, 300), (3, 200)] {
            let rec = photo(n, taken);
            for item in fetch_plan(Role::Keeper, &rec, &PlanContext::default()) {
                queue.push(QueuedFetch {
                    photo: rec.id,
                    taken_at: rec.taken_at,
                    item,
                });
            }
        }
        queue_order(&mut queue);
        let order: Vec<(BlobKind, u8)> =
            queue.iter().map(|q| (q.item.kind, q.photo.0[0])).collect();
        assert_eq!(
            order,
            vec![
                (BlobKind::Thumb, 2),
                (BlobKind::Thumb, 3),
                (BlobKind::Thumb, 1),
                (BlobKind::Preview, 2),
                (BlobKind::Preview, 3),
                (BlobKind::Preview, 1),
                (BlobKind::Original, 2),
                (BlobKind::Original, 3),
                (BlobKind::Original, 1),
            ]
        );
    }

    #[test]
    fn holders_rank_by_path_then_freshness_then_id() {
        let c = |d: u8, path, ago| Candidate {
            device: d,
            path,
            last_seen_ago_ms: ago,
        };
        let mut v = vec![
            c(5, PathQuality::Relay, Some(1)),
            c(4, PathQuality::Direct, Some(500)),
            c(3, PathQuality::Lan, Some(9_000)),
            c(2, PathQuality::Lan, Some(100)),
            c(1, PathQuality::Lan, Some(100)),
            c(6, PathQuality::Lan, None),
            c(7, PathQuality::Unknown, Some(0)),
        ];
        rank_holders(&mut v);
        assert_eq!(
            v.iter().map(|c| c.device).collect::<Vec<_>>(),
            vec![1, 2, 3, 6, 4, 5, 7]
        );
    }
}
