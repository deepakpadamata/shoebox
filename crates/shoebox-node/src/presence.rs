//! Gossip presence (M1-10): one iroh-gossip topic per library carries `Hello`, `Heartbeat` and
//! `NewItems` messages, giving a live "who is online" table and a push nudge for new photos.
//!
//! The topic is derived from the library id and root key, so devices of other libraries never
//! share it. Messages are unauthenticated in M1 (M2 signs them with the device certs): they carry
//! nothing sensitive (device name, role, app version), and a forged `NewItems` can at worst
//! trigger an extra sync with a peer we already know.

use crate::library_node::{Inner, LibraryNode};
use crate::node::NodeEvent;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::StreamExt;
use iroh_gossip::api::{Event, GossipSender};
use iroh_gossip::TopicId;
use serde::{Deserialize, Serialize};
use shoebox_core::ids::EndpointId;
use shoebox_core::model::Role;
use shoebox_core::store::{LibraryCapability, LibraryStore};
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
        device: EndpointId,
        name: String,
        role: Role,
        app_version: String,
    },
    Heartbeat {
        device: EndpointId,
    },
    NewItems {
        device: EndpointId,
        count: u32,
    },
}

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
}

fn topic_id(cap: &LibraryCapability) -> TopicId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"shoebox/presence/0");
    hasher.update(&cap.library.0);
    hasher.update(&cap.root.0);
    TopicId::from_bytes(*hasher.finalize().as_bytes())
}

fn encode(msg: &PresenceMsg) -> Bytes {
    Bytes::from(postcard::to_stdvec(msg).expect("presence message encodes"))
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
        let topic = self
            .node()
            .gossip()
            .subscribe(topic_id(&cap), bootstrap)
            .await
            .map_err(|e| anyhow!("joining presence topic: {e}"))?;
        let (sender, receiver) = topic.split();
        *self.inner.presence.lock().unwrap() = Some(PresenceState {
            sender: sender.clone(),
            cfg: cfg.clone(),
        });

        let weak = Arc::downgrade(&self.inner);
        let hello_sender = sender.clone();
        tokio::spawn(receive_loop(weak.clone(), receiver, hello_sender));
        tokio::spawn(heartbeat_loop(weak, sender, cfg));
        self.send_hello().await;
        Ok(())
    }

    async fn send_hello(&self) {
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
        let sender = self
            .inner
            .presence
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.sender.clone());
        if let Some(sender) = sender {
            let msg = PresenceMsg::NewItems {
                device: self.inner.device,
                count,
            };
            sender
                .broadcast(encode(&msg))
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

async fn send_hello<L: LibraryStore>(inner: &Arc<Inner<L>>) {
    let sender = inner
        .presence
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.sender.clone());
    let Some(sender) = sender else { return };
    let cfg = inner.node.config();
    let msg = PresenceMsg::Hello {
        device: inner.device,
        name: cfg.name,
        role: cfg.role,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if let Err(e) = sender.broadcast(encode(&msg)).await {
        debug!("could not send hello: {e}");
    }
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
                match postcard::from_bytes::<PresenceMsg>(&message.content) {
                    Ok(PresenceMsg::Hello {
                        device,
                        name,
                        role,
                        app_version,
                    }) => {
                        if device != inner.device {
                            touch(&inner, device, Some((name, role, app_version)));
                        }
                    }
                    Ok(PresenceMsg::Heartbeat { device }) => {
                        if device != inner.device {
                            touch(&inner, device, None);
                        }
                    }
                    Ok(PresenceMsg::NewItems { device, count }) => {
                        if device != inner.device {
                            touch(&inner, device, None);
                            debug!("{count} new item(s) announced by {device}: syncing");
                            let _ = inner.nudge.send(device);
                        }
                    }
                    Err(e) => debug!("ignoring malformed presence message: {e}"),
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
        let msg = PresenceMsg::Heartbeat {
            device: inner.device,
        };
        let _ = sender.broadcast(encode(&msg)).await;
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

    #[test]
    fn messages_round_trip_and_topics_are_per_library() {
        let msg = PresenceMsg::Hello {
            device: EndpointId([1; 32]),
            name: "Pi".into(),
            role: Role::Keeper,
            app_version: "0.1".into(),
        };
        assert_eq!(
            postcard::from_bytes::<PresenceMsg>(&encode(&msg)).unwrap(),
            msg
        );

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
        // Different library or root: different topic.
        assert_ne!(
            topic_id(&cap(1, 2, Capability::Write)),
            topic_id(&cap(9, 2, Capability::Write))
        );
        assert_ne!(
            topic_id(&cap(1, 2, Capability::Write)),
            topic_id(&cap(1, 3, Capability::Write))
        );
    }
}
