//! Fetch policy (M1-12): which blobs a device should have, as a pure function of its role.
//!
//! - **Keeper**: thumbnails, previews and every original.
//! - **Companion**: thumbnails and previews of everything; the original when the user pinned it,
//!   when it is recent (within the device's window), when it is a favourite (if the device keeps
//!   favourites), or when it is this device's own capture that does not yet have enough copies.
//!   Everything else lives on keepers.
//! - **Viewer**: thumbnails and previews only.
//! - A device other than a keeper that holds an original the app shows as it is (JPEG, PNG) does
//!   not fetch its preview: the viewer shows the original. Keepers keep every preview, as the
//!   source the other devices get them from.
//! - **Shared albums**: previews only, plus originals when the album's "keep originals" is on.
//!
//! - **Index shards** (S1-07): every device holds every live shard ([`index_plan`]), except that
//!   a Viewer of a library larger than [`VIEWER_VEC8_LIMIT`] leaves out the 8-bit vectors.
//!
//! The caller turns these plans into a queue ordered thumbnails → shards → previews → originals,
//! newest photos first ([`queue_order`]).

use crate::ids::{EndpointId, Timestamp};
use crate::model::{
    BlobKind, BlobRef, IndexShard, PhotoId, PhotoRecord, Role, ShardKind, SharedItem,
};

/// Above this many photos a Viewer keeps only the binary codes for search by meaning, not the
/// 8-bit vectors (epic S1 §7): somewhat less precise, still complete and offline.
pub const VIEWER_VEC8_LIMIT: usize = 250_000;

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
    /// This device holds the original already.
    pub holds_original: bool,
}

impl PlanContext {
    /// A Companion holds this original on its own account (not just because it is pinned).
    pub fn keeps_original(&self) -> bool {
        self.pinned || self.own_capture_below_target || self.recent || self.kept_favorite
    }
}

/// An original the app shows as it is, so a device holding it needs no preview. Other formats
/// (HEIC, camera raw) are not shown by every web view, and videos have no separate preview.
pub fn shows_directly(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/png")
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
    let preview_redundant =
        role != Role::Keeper && ctx.holds_original && shows_directly(&rec.original.mime);
    if !is_placeholder(&rec.original, &rec.preview) && !preview_redundant {
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

/// Index shards this device should hold, from the shards that count (or will once fetched).
pub fn index_plan<'a>(
    role: Role,
    library_size: usize,
    shards: impl IntoIterator<Item = &'a IndexShard>,
) -> Vec<FetchItem> {
    let skip_vec8 = role == Role::Viewer && library_size > VIEWER_VEC8_LIMIT;
    let mut plan = vec![];
    for s in shards {
        if skip_vec8 && s.kind == ShardKind::Vec8 {
            continue;
        }
        push_unique(&mut plan, BlobKind::Index, &s.blob);
    }
    plan
}

/// A device that shares the indexing backlog (an always-on indexer), weighted by its speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexCandidate {
    pub device: EndpointId,
    /// Photos per minute; 0 = takes no share.
    pub weight: u32,
}

/// Which candidate indexes `photo` for `kind` (epic S1 §5.2): weighted rendezvous hashing. Each
/// device scores `-ln(h) / weight`, `h` uniform in (0, 1) from BLAKE3(photo ‖ kind ‖ device), and
/// the lowest score wins (ties by device id). Shares follow the weights, and adding or removing
/// a device moves only that device's share. Every device computes the same answer.
pub fn index_owner(
    photo: &PhotoId,
    kind: ShardKind,
    candidates: &[IndexCandidate],
) -> Option<EndpointId> {
    candidates
        .iter()
        .filter(|c| c.weight > 0)
        .map(|c| {
            let mut h = blake3::Hasher::new();
            h.update(&photo.0);
            h.update(&[kind.byte()]);
            h.update(&c.device.0);
            let x = u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().expect("8"));
            let u = (x as f64 + 0.5) / 18_446_744_073_709_551_616.0;
            (-u.ln() / f64::from(c.weight), c.device)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, d)| d)
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
        // Shards right after thumbnails: a new device can search soon after it shows the grid.
        BlobKind::Index => 1,
        BlobKind::Preview => 2,
        BlobKind::Original => 3,
    }
}

/// Order a queue: all thumbnails, then index shards, then previews, then originals; within a kind, newest
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
                    holds_original: false,
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
    fn a_device_holding_an_original_it_can_show_skips_the_preview_but_a_keeper_does_not() {
        let held = PlanContext {
            recent: true,
            holds_original: true,
            ..PlanContext::default()
        };
        assert_eq!(
            kinds(&fetch_plan(Role::Companion, &photo(1, 0), &held)),
            vec![BlobKind::Thumb, BlobKind::Original]
        );
        assert_eq!(
            kinds(&fetch_plan(Role::Keeper, &photo(1, 0), &held)),
            vec![BlobKind::Thumb, BlobKind::Preview, BlobKind::Original],
            "keepers are where the other devices get previews from"
        );
        let mut heic = photo(1, 0);
        heic.original.mime = "image/heic".into();
        assert!(
            kinds(&fetch_plan(Role::Companion, &heic, &held)).contains(&BlobKind::Preview),
            "not every web view shows HEIC: the preview stays"
        );
        let fetching = PlanContext {
            holds_original: false,
            ..held
        };
        assert!(
            kinds(&fetch_plan(Role::Companion, &photo(1, 0), &fetching))
                .contains(&BlobKind::Preview),
            "until the original is here the preview is what shows"
        );
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

    fn shard(n: u8, kind: ShardKind) -> IndexShard {
        IndexShard {
            blob: BlobRef {
                hash: Blake3Hash([n; 32]),
                size: 10,
                mime: "application/x-shoebox-index".into(),
            },
            model: crate::model::ModelId([1; 32]),
            kind,
            count: 1,
            replaces: vec![],
            made_by: EndpointId([1; 32]),
            hlc: Hlc::GENESIS,
        }
    }

    #[test]
    fn shards_come_after_thumbnails_and_before_previews() {
        let rec = photo(1, 100);
        let mut queue: Vec<QueuedFetch> = fetch_plan(Role::Keeper, &rec, &PlanContext::default())
            .into_iter()
            .map(|item| QueuedFetch {
                photo: rec.id,
                taken_at: rec.taken_at,
                item,
            })
            .collect();
        for item in index_plan(Role::Keeper, 1, &[shard(50, ShardKind::Vec8)]) {
            queue.insert(
                0,
                QueuedFetch {
                    photo: item.blob.hash,
                    taken_at: Timestamp(0),
                    item,
                },
            );
        }
        queue_order(&mut queue);
        let kinds: Vec<BlobKind> = queue.iter().map(|q| q.item.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlobKind::Thumb,
                BlobKind::Index,
                BlobKind::Preview,
                BlobKind::Original
            ]
        );
    }

    #[test]
    fn a_viewer_of_a_huge_library_keeps_binary_codes_only() {
        let shards = [
            shard(1, ShardKind::Vec8),
            shard(2, ShardKind::Vec1),
            shard(3, ShardKind::Text),
            shard(4, ShardKind::Faces),
        ];
        let held = |role, size| -> Vec<u8> {
            index_plan(role, size, &shards)
                .iter()
                .map(|i| i.blob.hash.0[0])
                .collect()
        };
        assert_eq!(held(Role::Viewer, 300_000), vec![2, 3, 4]);
        assert_eq!(held(Role::Viewer, 100_000), vec![1, 2, 3, 4]);
        assert_eq!(held(Role::Companion, 300_000), vec![1, 2, 3, 4]);
        assert_eq!(held(Role::Keeper, 300_000), vec![1, 2, 3, 4]);
    }

    #[test]
    fn rendezvous_follows_weights_and_moves_little() {
        let dev = |n: u8| EndpointId([n; 32]);
        let cands = [
            IndexCandidate {
                device: dev(1),
                weight: 10,
            },
            IndexCandidate {
                device: dev(2),
                weight: 20,
            },
            IndexCandidate {
                device: dev(3),
                weight: 40,
            },
        ];
        let photos: Vec<PhotoId> = (0..7_000u32)
            .map(|i| crate::ids::Blake3Hash::of_bytes(&i.to_le_bytes()))
            .collect();
        let owners: Vec<EndpointId> = photos
            .iter()
            .map(|p| index_owner(p, ShardKind::Vec8, &cands).unwrap())
            .collect();
        for (d, want) in [(1u8, 1.0 / 7.0), (2, 2.0 / 7.0), (3, 4.0 / 7.0)] {
            let share = owners.iter().filter(|o| **o == dev(d)).count() as f64 / 7_000.0;
            assert!(
                (share / want - 1.0).abs() < 0.1,
                "device {d}: {share:.3} vs {want:.3}"
            );
        }
        // Device 2 leaves: only its photos move.
        let without: Vec<EndpointId> = photos
            .iter()
            .map(|p| index_owner(p, ShardKind::Vec8, &[cands[0], cands[2]]).unwrap())
            .collect();
        for (a, b) in owners.iter().zip(&without) {
            assert!(a == b || *a == dev(2));
        }
        // Another kind is shared out independently, and no candidate means nobody.
        let other = photos
            .iter()
            .filter(|p| {
                index_owner(p, ShardKind::Faces, &cands) != index_owner(p, ShardKind::Vec8, &cands)
            })
            .count();
        assert!(other > 1_000);
        assert_eq!(index_owner(&photos[0], ShardKind::Vec8, &[]), None);
        assert_eq!(
            index_owner(
                &photos[0],
                ShardKind::Vec8,
                &[IndexCandidate {
                    device: dev(1),
                    weight: 0
                }]
            ),
            None
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
