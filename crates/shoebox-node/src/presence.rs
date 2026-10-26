//! Gossip presence (M1-10): one iroh-gossip topic per library carries `Hello`, `Heartbeat` and
//! `NewItems` messages, giving a live "who is online" table and a push nudge for new photos.
//!
//! The topic is derived from the library's identity id, so devices of other libraries never
//! share it. Gossip relays messages through other peers, and the gossip protocol is
//! open to devices outside the device list (shared albums need it), so every message is signed
//! with the sender's device key, bound to this topic, and accepted only from a listed, non-revoked
//! device (or, before a library has any device list, from whoever signed it, the same rule as the
//! connection gate). A per-device send time that must increase stops replays of old messages.

use crate::library_node::{Inner, LibraryNode};
use crate::node::NodeEvent;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::StreamExt;
use iroh_gossip::api::{Event, GossipSender};
use iroh_gossip::TopicId;
use serde::{Deserialize, Serialize};
use shoebox_core::identity::{verify_signature, KeyPair};
use shoebox_core::ids::EndpointId;
use shoebox_core::model::Role;
use shoebox_core::store::{LibraryCapability, LibraryStore, SignedDeviceList};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

#[derive(Clone, Debug)]
pub struct PresenceConfig {
    /// How often this device announces it is alive.
    pub heartbeat: Duration,
    /// A device not heard from for this long is shown as offline.
    pub offline_after: Duration,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(10),
            offline_after: Duration::from_secs(35),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) enum PresenceMsg {
    Hello {
        name: String,
        role: Role,
        app_version: String,
    },
    Heartbeat,
    NewItems {
        count: u32,
    },
}

/// What the sender signs: who it is, when it sent this (its own clock, ms), and the message.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct PresenceBody {
    device: EndpointId,
    sent_at_ms: u64,
    msg: PresenceMsg,
}

/// On the wire: the encoded body and the device key's signature over it.
#[derive(Serialize, Deserialize)]
struct Envelope {
    body: Vec<u8>,
    sig: Vec<u8>,
}

/// Longest name and version kept from a `Hello`; anything longer is cut.
const MAX_NAME_CHARS: usize = 64;
const MAX_VERSION_CHARS: usize = 32;

/// A device of this library, as last seen on the presence topic.
#[derive(Clone, Debug)]
pub struct DeviceStatus {
    pub id: EndpointId,
    pub name: Option<String>,
    pub role: Option<Role>,
    pub app_version: Option<String>,
    pub online: bool,
    pub last_seen: Option<Instant>,
}

pub(crate) struct DeviceRecord {
    name: Option<String>,
    role: Option<Role>,
    app_version: Option<String>,
    last_seen: Instant,
    online: bool,
}

pub(crate) struct PresenceState {
    sender: GossipSender,
    cfg: PresenceConfig,
    topic: TopicId,
    /// The latest send time accepted per device; anything not newer is a replay.
    last_sent: HashMap<EndpointId, u64>,
    /// The send time of this device's previous message: the next one is always later, even when
    /// two go out in the same millisecond or the clock steps back.
    own_last_sent: u64,
}

fn topic_id(cap: &LibraryCapability) -> TopicId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shoebox/presence/2");
    // The identity id, not the epoch or the root key: presence survives a rotation (M2-03b) and
    // a root handover. Knowing the topic gains nothing: messages are signed by listed devices.
    hasher.update(&cap.identity_id().0);
    TopicId::from_bytes(*hasher.finalize().as_bytes())
}

/// The bytes a signature covers: a domain tag, the topic (so a message cannot be replayed into
/// another library's topic) and the encoded body.
fn signed_bytes(topic: &TopicId, body: &[u8]) -> Vec<u8> {
    let mut out = b"shoebox/presence-msg/1\0".to_vec();
    out.extend_from_slice(topic.as_bytes());
    out.extend_from_slice(body);
    out
}

fn seal(key: &KeyPair, topic: &TopicId, sent_at_ms: u64, msg: PresenceMsg) -> Bytes {
    let body = postcard::to_stdvec(&PresenceBody {
        device: key.endpoint_id(),
        sent_at_ms,
        msg,
    })
    .expect("presence body encodes");
    let sig = key.sign(&signed_bytes(topic, &body));
    Bytes::from(postcard::to_stdvec(&Envelope { body, sig }).expect("presence envelope encodes"))
}

/// Decode and check a received message: well formed, signed by the device it names, for this
/// topic. Says nothing yet about whether that device belongs to the library.
fn open(topic: &TopicId, bytes: &[u8]) -> Option<PresenceBody> {
    let env: Envelope = postcard::from_bytes(bytes).ok()?;
    let body: PresenceBody = postcard::from_bytes(&env.body).ok()?;
    verify_signature(&body.device.0, &signed_bytes(topic, &env.body), &env.sig).then_some(body)
}

/// Whether a signed message from `device` may change what this device shows: it must be a
/// listed, non-revoked device. A library without any device list yet accepts every signer
/// (as the connection gate does, A-061).
fn from_member(list: &SignedDeviceList, device: &EndpointId) -> bool {
    list.devices.is_empty()
        || list
            .devices
            .iter()
            .any(|d| d.device == *device && !d.revoked)
}

/// Record `sent_at_ms` as the newest message from `device`, unless it is not newer than one
/// already accepted (a replay, or a duplicate).
fn is_fresh(last_sent: &mut HashMap<EndpointId, u64>, device: EndpointId, sent_at_ms: u64) -> bool {
    match last_sent.get(&device) {
        Some(&last) if sent_at_ms <= last => false,
        _ => {
            last_sent.insert(device, sent_at_ms);
            true
        }
    }
}

fn truncate(s: String, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => s[..i].to_string(),
        None => s,
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Join this library's presence topic and start announcing. Needs
    /// [`LibraryNode::with_capability`] (the topic is derived from the library).
    pub async fn start_presence(&self, cfg: PresenceConfig) -> Result<()> {
        let cap = self.inner.cap.lock().unwrap().clone().ok_or_else(|| {
            anyhow!("presence needs the library capability (use with_capability)")
        })?;
        let bootstrap: Vec<iroh::EndpointId> = self
            .peers()
            .iter()
            .filter_map(|id| crate::addr::to_iroh_id(id).ok())
            .collect();
        let topic_id = topic_id(&cap);
        let topic = self
            .node()
            .gossip()
            .subscribe(topic_id, bootstrap)
            .await
            .map_err(|e| anyhow!("joining presence topic: {e}"))?;
        let (sender, receiver) = topic.split();
        *self.inner.presence.lock().unwrap() = Some(PresenceState {
            sender: sender.clone(),
            cfg: cfg.clone(),
            topic: topic_id,
            last_sent: HashMap::new(),
            own_last_sent: 0,
        });

        let weak = Arc::downgrade(&self.inner);
        let hello_sender = sender.clone();
        tokio::spawn(receive_loop(weak.clone(), receiver, hello_sender));
        tokio::spawn(heartbeat_loop(weak, sender, cfg));
        self.send_hello().await;
        Ok(())
    }

    pub(crate) async fn send_hello(&self) {
        send_hello(&self.inner).await;
    }

    /// Everyone else in this library that has announced itself, with online state.
    pub fn devices(&self) -> Vec<DeviceStatus> {
        let offline_after = self
            .inner
            .presence
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.cfg.offline_after)
            .unwrap_or(PresenceConfig::default().offline_after);
        let table = self.inner.devices.lock().unwrap();
        let mut out: Vec<DeviceStatus> = table
            .iter()
            .map(|(id, r)| DeviceStatus {
                id: *id,
                name: r.name.clone(),
                role: r.role,
                app_version: r.app_version.clone(),
                online: r.online && r.last_seen.elapsed() < offline_after,
                last_seen: Some(r.last_seen),
            })
            .collect();
        out.sort_by_key(|d| d.id);
        out
    }

    /// Tell the other devices that this one has new items, so they sync now instead of waiting.
    pub async fn announce_new_items(&self, count: u32) -> Result<()> {
        if let Some((sender, bytes)) = sealed(&self.inner, PresenceMsg::NewItems { count }) {
            sender
                .broadcast(bytes)
                .await
                .map_err(|e| anyhow!("announcing new items: {e}"))?;
        }
        Ok(())
    }

    /// Make the presence topic reach a newly added peer.
    pub(crate) fn presence_join(&self, peer: &EndpointId) {
        let sender = self
            .inner
            .presence
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.sender.clone());
        if let (Some(sender), Ok(iroh_id)) = (sender, crate::addr::to_iroh_id(peer)) {
            tokio::spawn(async move {
                let _ = sender.join_peers(vec![iroh_id]).await;
            });
        }
    }
}

/// `msg` signed by this device for the presence topic, with the sender to broadcast it on; `None`
/// before presence has started.
fn sealed<L: LibraryStore>(inner: &Inner<L>, msg: PresenceMsg) -> Option<(GossipSender, Bytes)> {
    let (sender, topic, sent_at_ms) = {
        let mut presence = inner.presence.lock().unwrap();
        let p = presence.as_mut()?;
        p.own_last_sent = inner.clock.now_ms().max(p.own_last_sent + 1);
        (p.sender.clone(), p.topic, p.own_last_sent)
    };
    let bytes = seal(&inner.node.device_key(), &topic, sent_at_ms, msg);
    Some((sender, bytes))
}

async fn send_hello<L: LibraryStore>(inner: &Arc<Inner<L>>) {
    let cfg = inner.node.config();
    let msg = PresenceMsg::Hello {
        name: cfg.name,
        role: cfg.role,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let Some((sender, bytes)) = sealed(inner, msg) else {
        return;
    };
    if let Err(e) = sender.broadcast(bytes).await {
        debug!("could not send hello: {e}");
    }
}

/// Check a received message and, if it passes, the device it came from.
async fn accept<L: LibraryStore>(inner: &Inner<L>, bytes: &[u8]) -> Option<PresenceBody> {
    let topic = inner.presence.lock().unwrap().as_ref()?.topic;
    let Some(body) = open(&topic, bytes) else {
        debug!("ignoring a presence message that is malformed or not signed by its device");
        return None;
    };
    if body.device == inner.device {
        return None;
    }
    match inner.store.device_list().await {
        Ok(list) if from_member(&list, &body.device) => {}
        Ok(_) => {
            debug!(
                "ignoring presence from {}: not a device of this library",
                body.device
            );
            return None;
        }
        Err(e) => {
            debug!("ignoring presence: device list unavailable: {e}");
            return None;
        }
    }
    let mut presence = inner.presence.lock().unwrap();
    let state = presence.as_mut()?;
    is_fresh(&mut state.last_sent, body.device, body.sent_at_ms).then_some(body)
}

fn set_online<L: LibraryStore>(inner: &Inner<L>, id: EndpointId, online: bool) {
    let changed = {
        let mut table = inner.devices.lock().unwrap();
        match table.get_mut(&id) {
            Some(r) if r.online != online => {
                r.online = online;
                true
            }
            _ => false,
        }
    };
    if changed {
        inner
            .node
            .emit(NodeEvent::PresenceChanged { device: id, online });
    }
}

fn touch<L: LibraryStore>(inner: &Inner<L>, id: EndpointId, hello: Option<(String, Role, String)>) {
    let was_online = {
        let mut table = inner.devices.lock().unwrap();
        let rec = table.entry(id).or_insert_with(|| DeviceRecord {
            name: None,
            role: None,
            app_version: None,
            last_seen: Instant::now(),
            online: false,
        });
        rec.last_seen = Instant::now();
        if let Some((name, role, version)) = hello {
            rec.name = Some(name);
            rec.role = Some(role);
            rec.app_version = Some(version);
        }
        std::mem::replace(&mut rec.online, true)
    };
    if !was_online {
        inner.node.emit(NodeEvent::PresenceChanged {
            device: id,
            online: true,
        });
    }
}

async fn receive_loop<L: LibraryStore>(
    weak: Weak<Inner<L>>,
    mut receiver: iroh_gossip::api::GossipReceiver,
    _sender: GossipSender,
) {
    while let Some(event) = receiver.next().await {
        let Some(inner) = weak.upgrade() else { break };
        match event {
            Ok(Event::Received(message)) => {
                let Some(PresenceBody { device, msg, .. }) = accept(&inner, &message.content).await
                else {
                    continue;
                };
                match msg {
                    PresenceMsg::Hello {
                        name,
                        role,
                        app_version,
                    } => {
                        let hello = (
                            truncate(name, MAX_NAME_CHARS),
                            role,
                            truncate(app_version, MAX_VERSION_CHARS),
                        );
                        touch(&inner, device, Some(hello));
                    }
                    PresenceMsg::Heartbeat => touch(&inner, device, None),
                    PresenceMsg::NewItems { count } => {
                        touch(&inner, device, None);
                        debug!("{count} new item(s) announced by {device}: syncing");
                        let _ = inner.nudge.send(device);
                    }
                }
            }
            // A new neighbor does not know us yet: introduce ourselves.
            Ok(Event::NeighborUp(_)) => send_hello(&inner).await,
            Ok(Event::NeighborDown(id)) => {
                set_online(&inner, crate::addr::from_iroh_id(&id), false)
            }
            Ok(Event::Lagged) => warn!("presence receiver lagged"),
            Err(e) => {
                debug!("presence stream ended: {e}");
                break;
            }
        }
    }
}

async fn heartbeat_loop<L: LibraryStore>(
    weak: Weak<Inner<L>>,
    sender: GossipSender,
    cfg: PresenceConfig,
) {
    let mut ticker = tokio::time::interval(cfg.heartbeat);
    loop {
        ticker.tick().await;
        let Some(inner) = weak.upgrade() else { break };
        if let Some((_, bytes)) = sealed(&inner, PresenceMsg::Heartbeat) {
            let _ = sender.broadcast(bytes).await;
        }
        // Mark devices we have not heard from as offline.
        let stale: Vec<EndpointId> = inner
            .devices
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.online && r.last_seen.elapsed() >= cfg.offline_after)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            set_online(&inner, id, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoebox_core::store::{Capability, LibraryId, RootPublicKey};

    fn hello() -> PresenceMsg {
        PresenceMsg::Hello {
            name: "Pi".into(),
            role: Role::Keeper,
            app_version: "0.1".into(),
        }
    }

    #[test]
    fn signed_messages_open_only_for_their_signer_and_topic() {
        let key = KeyPair::generate();
        let topic = TopicId::from_bytes([7; 32]);
        let bytes = seal(&key, &topic, 42, hello());
        let body = open(&topic, &bytes).expect("opens");
        assert_eq!(body.device, key.endpoint_id());
        assert_eq!(body.sent_at_ms, 42);
        assert_eq!(body.msg, hello());

        // Another library's topic: the signature does not cover it.
        assert!(open(&TopicId::from_bytes([8; 32]), &bytes).is_none());

        // A body edited after signing (here: claiming another device) fails.
        let mut env: Envelope = postcard::from_bytes(&bytes).unwrap();
        let mut forged: PresenceBody = postcard::from_bytes(&env.body).unwrap();
        forged.device = EndpointId([9; 32]);
        env.body = postcard::to_stdvec(&forged).unwrap();
        assert!(open(&topic, &postcard::to_stdvec(&env).unwrap()).is_none());

        // Signed by one key while naming another device: fails.
        let other = KeyPair::generate();
        let body = postcard::to_stdvec(&PresenceBody {
            device: key.endpoint_id(),
            sent_at_ms: 43,
            msg: PresenceMsg::Heartbeat,
        })
        .unwrap();
        let sig = other.sign(&signed_bytes(&topic, &body));
        let env = postcard::to_stdvec(&Envelope { body, sig }).unwrap();
        assert!(open(&topic, &env).is_none());

        // Unsigned old-format or garbage bytes: ignored, never a panic.
        assert!(open(&topic, b"").is_none());
        assert!(open(&topic, &[0xff; 300]).is_none());
    }

    #[test]
    fn only_listed_devices_count_once_a_device_list_exists() {
        use shoebox_core::model::DeviceCert;
        let cert = |b: u8, revoked: bool| DeviceCert {
            device: EndpointId([b; 32]),
            role: Role::Keeper,
            name: String::new(),
            added_by: EndpointId([0; 32]),
            hlc: shoebox_core::hlc::Hlc::GENESIS,
            revoked,
            signature: vec![],
        };
        let empty = SignedDeviceList {
            admins: vec![],
            devices: vec![],
        };
        assert!(from_member(&empty, &EndpointId([1; 32])));
        let list = SignedDeviceList {
            admins: vec![],
            devices: vec![cert(1, false), cert(2, true)],
        };
        assert!(from_member(&list, &EndpointId([1; 32])));
        assert!(!from_member(&list, &EndpointId([2; 32])), "revoked");
        assert!(!from_member(&list, &EndpointId([3; 32])), "not listed");
    }

    #[test]
    fn replays_and_duplicates_are_dropped() {
        let mut last = HashMap::new();
        let a = EndpointId([1; 32]);
        assert!(is_fresh(&mut last, a, 100));
        assert!(!is_fresh(&mut last, a, 100), "duplicate");
        assert!(!is_fresh(&mut last, a, 50), "older");
        assert!(is_fresh(&mut last, a, 101));
        assert!(is_fresh(&mut last, EndpointId([2; 32]), 1), "per device");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        assert_eq!(truncate("Pi".into(), 64), "Pi");
        assert_eq!(truncate("é".repeat(70), 64).chars().count(), 64);
    }

    #[test]
    fn topics_are_per_library() {
        let cap = |lib: u8, root: u8, mode| LibraryCapability {
            library: LibraryId([lib; 16]),
            root: RootPublicKey([root; 32]),
            mode,
            backend: vec![lib],
        };
        // Same library, different access level or secret material: same topic.
        assert_eq!(
            topic_id(&cap(1, 2, Capability::Write)),
            topic_id(&cap(1, 2, Capability::Read))
        );
        // Different library: different topic. A new root key (a handover) keeps the topic.
        assert_ne!(
            topic_id(&cap(1, 2, Capability::Write)),
            topic_id(&cap(9, 2, Capability::Write))
        );
        assert_eq!(
            topic_id(&cap(1, 2, Capability::Write)),
            topic_id(&cap(1, 3, Capability::Write))
        );
    }
}
