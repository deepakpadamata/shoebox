//! `AlbumState`: the merged in-memory view of one shared-album replica (see [`crate::library_state`]
//! for the same pattern on the personal library).

use crate::hlc::{Hlc, HlcClock};
use crate::ids::EndpointId;
use crate::model::*;
use crate::store::*;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// A replicated record of a shared album.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumEntry {
    Meta(SharedAlbumMeta),
    Item(SharedItem),
    Member { hlc: Hlc, member: Member },
    Event(AlbumEvent),
}

/// Which record a write changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlbumTouched {
    Item(PhotoId),
    Member(EndpointId),
    Event(Hlc),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AlbumEffect {
    pub touched: Vec<AlbumTouched>,
}

impl AlbumEffect {
    pub fn changed(&self) -> bool {
        !self.touched.is_empty()
    }
    fn touch(&mut self, t: AlbumTouched) {
        if !self.touched.contains(&t) {
            self.touched.push(t);
        }
    }
}

#[derive(Clone)]
pub struct AlbumState {
    me: EndpointId,
    meta: SharedAlbumMeta,
    items: BTreeMap<PhotoId, SharedItem>,
    /// Per member the entry with the greatest `(hlc, name, role)` wins.
    members: BTreeMap<EndpointId, (Hlc, Member)>,
    events: BTreeMap<Hlc, AlbumEvent>,
    hlc: HlcClock,
    max_seen: Hlc,
}

fn item_key(i: &SharedItem) -> impl Ord {
    (
        i.added_at,
        i.added_by,
        i.original.clone(),
        i.shared_hash,
        i.preview.clone(),
        i.thumb.clone(),
        i.taken_at,
    )
}

fn member_key(m: &(Hlc, Member)) -> impl Ord + '_ {
    (m.0, &m.1.display_name, m.1.role)
}

impl AlbumState {
    pub fn new(meta: SharedAlbumMeta, device: EndpointId) -> Self {
        Self {
            me: device,
            meta,
            items: BTreeMap::new(),
            members: BTreeMap::new(),
            events: BTreeMap::new(),
            hlc: HlcClock::new(device),
            max_seen: Hlc {
                physical_ms: 0,
                counter: 0,
                device,
            },
        }
    }

    /// A copy of this replica's data for `device` (what a joiner receives from the inviter).
    pub fn fork_for(&self, device: EndpointId) -> Self {
        let mut s = self.clone();
        s.me = device;
        s.hlc = HlcClock::new(device);
        s.max_seen = Hlc {
            device,
            ..self.max_seen
        };
        s
    }

    pub fn device(&self) -> EndpointId {
        self.me
    }

    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        let seen = self.max_seen;
        let h = self.hlc.observe(now_ms, &seen);
        self.max_seen = self.max_seen.max(h);
        h
    }

    pub fn role_of(&self, device: EndpointId) -> Option<MemberRole> {
        self.members.get(&device).map(|(_, m)| m.role)
    }

    pub fn my_role(&self) -> Option<MemberRole> {
        self.role_of(self.me)
    }

    fn put_member(&mut self, hlc: Hlc, member: Member, fx: &mut AlbumEffect) {
        self.max_seen = self.max_seen.max(hlc);
        let candidate = (hlc, member);
        match self.members.get(&candidate.1.key) {
            Some(existing) if member_key(existing) >= member_key(&candidate) => {}
            _ => {
                fx.touch(AlbumTouched::Member(candidate.1.key));
                self.members.insert(candidate.1.key, candidate);
            }
        }
    }

    fn put_event(&mut self, e: &AlbumEvent, fx: &mut AlbumEffect) {
        self.max_seen = self.max_seen.max(e.hlc);
        if self.events.insert(e.hlc, e.clone()).as_ref() != Some(e) {
            fx.touch(AlbumTouched::Event(e.hlc));
        }
    }

    fn put_item(&mut self, item: &SharedItem, fx: &mut AlbumEffect) -> bool {
        let is_new = !self.items.contains_key(&item.id);
        match self.items.get(&item.id) {
            Some(existing) if item_key(existing) <= item_key(item) => {}
            _ => {
                self.items.insert(item.id, item.clone());
                fx.touch(AlbumTouched::Item(item.id));
            }
        }
        is_new
    }

    /// Register this replica's device as a member with `role`; non-creators log a join event.
    pub fn install_member(&mut self, role: MemberRole, now_ms: u64) -> AlbumEffect {
        let mut fx = AlbumEffect::default();
        let hlc = self.tick(now_ms);
        let member = Member {
            key: self.me,
            display_name: format!("{}", self.me),
            role,
        };
        self.put_member(hlc, member, &mut fx);
        if role != MemberRole::Owner {
            let h = self.tick(now_ms);
            let ev = AlbumEvent {
                hlc: h,
                member: self.me,
                kind: AlbumEventKind::MemberJoined,
            };
            self.put_event(&ev, &mut fx);
        }
        fx
    }

    /// Add items (first writer by `(added_at, added_by)` wins; re-adding is a no-op) and log an
    /// `ItemsAdded` event for the genuinely new ones. Needs a Contributor/Owner role.
    pub fn add_items(&mut self, items: &[SharedItem], now_ms: u64) -> Result<AlbumEffect> {
        match self.my_role() {
            Some(MemberRole::Owner | MemberRole::Contributor) => {}
            _ => return Err(StoreError::ReadOnly),
        }
        let mut fx = AlbumEffect::default();
        let mut new = 0u32;
        for item in items {
            if self.put_item(item, &mut fx) {
                new += 1;
            }
        }
        if new > 0 {
            let hlc = self.tick(now_ms);
            let ev = AlbumEvent {
                hlc,
                member: self.me,
                kind: AlbumEventKind::ItemsAdded { count: new },
            };
            self.put_event(&ev, &mut fx);
        }
        Ok(fx)
    }

    pub fn meta(&self) -> SharedAlbumMeta {
        self.meta.clone()
    }

    pub fn items(&self, page: Page) -> Vec<SharedItem> {
        let mut items: Vec<&SharedItem> = self.items.values().collect();
        items.sort_by_key(|i| (Reverse(i.added_at), i.id));
        let skip = page
            .after
            .and_then(|c| items.iter().position(|i| i.id == c.id).map(|p| p + 1))
            .unwrap_or(0);
        items
            .into_iter()
            .skip(skip)
            .take(page.limit)
            .cloned()
            .collect()
    }

    pub fn members(&self) -> Vec<Member> {
        self.members.values().map(|(_, m)| m.clone()).collect()
    }

    pub fn events(&self, since: Option<Hlc>) -> Vec<AlbumEvent> {
        self.events
            .values()
            .filter(|e| since.is_none_or(|h| e.hlc > h))
            .cloned()
            .collect()
    }

    // ---------- replication ----------

    pub fn apply_entry(&mut self, entry: &AlbumEntry) -> AlbumEffect {
        let mut fx = AlbumEffect::default();
        match entry {
            AlbumEntry::Meta(_) => {} // immutable once created; replicas agree by construction
            AlbumEntry::Item(item) => {
                self.put_item(item, &mut fx);
            }
            AlbumEntry::Member { hlc, member } => self.put_member(*hlc, member.clone(), &mut fx),
            AlbumEntry::Event(e) => self.put_event(e, &mut fx),
        }
        fx
    }

    pub fn entry(&self, what: &AlbumTouched) -> Option<AlbumEntry> {
        match what {
            AlbumTouched::Item(id) => self.items.get(id).cloned().map(AlbumEntry::Item),
            AlbumTouched::Member(k) => self.members.get(k).map(|(hlc, m)| AlbumEntry::Member {
                hlc: *hlc,
                member: m.clone(),
            }),
            AlbumTouched::Event(h) => self.events.get(h).cloned().map(AlbumEntry::Event),
        }
    }

    pub fn all_entries(&self) -> Vec<AlbumEntry> {
        let mut out = vec![AlbumEntry::Meta(self.meta.clone())];
        out.extend(self.items.values().cloned().map(AlbumEntry::Item));
        out.extend(self.members.values().map(|(h, m)| AlbumEntry::Member {
            hlc: *h,
            member: m.clone(),
        }));
        out.extend(self.events.values().cloned().map(AlbumEntry::Event));
        out
    }

    pub fn merge_from(&mut self, other: &AlbumState) -> AlbumEffect {
        let mut fx = AlbumEffect::default();
        for entry in other.all_entries() {
            for t in self.apply_entry(&entry).touched {
                fx.touch(t);
            }
        }
        self.max_seen = self.max_seen.max(other.max_seen);
        fx
    }
}
