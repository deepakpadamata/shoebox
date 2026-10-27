//! Loro replication over iroh (`shoebox/loro/0`, see [`crate::wire`]).

use crate::library::LibraryShared;
use crate::wire::{read_msg, write_msg, Msg};
use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use loro::VersionVector;
use shoebox_core::store::{LibraryId, PeerAddr, StoreError};
use shoebox_node::{from_iroh_id, to_endpoint_addr, ConnHandler, Node, LORO_ALPN};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Extra rounds when either side wrote while a sync was running.
const MAX_ROUNDS: usize = 4;

pub(crate) struct IrohNet {
    node: Node,
    libs: Arc<Mutex<HashMap<LibraryId, Weak<LibraryShared>>>>,
}

fn decode_vv(bytes: &[u8]) -> Result<VersionVector> {
    VersionVector::decode(bytes).map_err(|e| anyhow!("bad version vector: {e}"))
}

impl IrohNet {
    /// Create the transport for `node` and start answering Loro syncs on it.
    pub(crate) fn new(node: Node) -> Arc<Self> {
        let net = Arc::new(Self {
            node: node.clone(),
            libs: Arc::new(Mutex::new(HashMap::new())),
        });
        let handler: Arc<dyn ConnHandler> = net.clone();
        *node.loro_slot().lock().unwrap() = Some(handler);
        net
    }

    /// A replica moved to a new epoch id.
    pub(crate) fn rotate(&self, old: LibraryId, new: LibraryId, shared: &Arc<LibraryShared>) {
        let mut libs = self.libs.lock().unwrap();
        libs.remove(&old);
        libs.insert(new, Arc::downgrade(shared));
    }

    pub(crate) fn register(&self, library: LibraryId, shared: &Arc<LibraryShared>) {
        self.libs
            .lock()
            .unwrap()
            .insert(library, Arc::downgrade(shared));
    }

    /// Reconcile `shared` with `peer`, retrying while writes keep racing the exchange.
    pub(crate) async fn sync(
        &self,
        shared: &Arc<LibraryShared>,
        library: LibraryId,
        peer: &PeerAddr,
    ) -> Result<(), StoreError> {
        let addr = to_endpoint_addr(peer)?;
        let run = async {
            let conn = self
                .node
                .endpoint()
                .connect(addr, LORO_ALPN)
                .await
                .context("connecting")?;
            let mut converged = false;
            for _ in 0..MAX_ROUNDS {
                if round(&conn, shared, library).await? {
                    converged = true;
                    break;
                }
            }
            conn.close(0u32.into(), b"done");
            ensure!(converged, "replicas kept changing during the sync");
            Ok::<(), anyhow::Error>(())
        };
        match tokio::time::timeout(Duration::from_secs(120), run).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(StoreError::Backend(format!("{e:#}"))),
            Err(_) => Err(StoreError::Unreachable(peer.id)),
        }
    }
}

/// One exchange on a fresh stream; `true` if both vectors ended up equal.
async fn round(conn: &Connection, shared: &Arc<LibraryShared>, library: LibraryId) -> Result<bool> {
    let (mut send, mut recv) = conn.open_bi().await?;
    write_msg(
        &mut send,
        &Msg::Hello {
            library,
            vv: shared.oplog_vv_in(library)?.encode(),
        },
    )
    .await?;
    let Msg::Reply { updates, vv } = read_msg(&mut recv).await? else {
        bail!("unexpected reply");
    };
    let theirs = decode_vv(&vv)?;
    shared.import_bytes_in(library, &updates)?;
    write_msg(
        &mut send,
        &Msg::Updates(shared.updates_since_in(library, &theirs)?),
    )
    .await?;
    send.finish()?;
    let Msg::Done { vv } = read_msg(&mut recv).await? else {
        bail!("unexpected end of exchange");
    };
    Ok(decode_vv(&vv)? == shared.oplog_vv_in(library)?)
}

impl ConnHandler for IrohNet {
    fn handle(&self, conn: Connection) -> BoxFuture<'static, Result<()>> {
        let libs = self.libs.clone();
        Box::pin(async move {
            let peer = from_iroh_id(&conn.remote_id());
            loop {
                let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                    return Ok(()); // the client is done
                };
                let result = serve_round(&libs, &mut send, &mut recv).await;
                if let Err(e) = result {
                    let _ = write_msg(&mut send, &Msg::Error(format!("{e:#}"))).await;
                    let _ = send.finish();
                    tracing::debug!(%peer, "loro sync failed: {e:#}");
                    return Ok(());
                }
                let _ = send.finish();
            }
        })
    }
}

async fn serve_round(
    libs: &Mutex<HashMap<LibraryId, Weak<LibraryShared>>>,
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<()> {
    let Msg::Hello { library, vv } = read_msg(recv).await? else {
        bail!("expected hello");
    };
    let shared = libs
        .lock()
        .unwrap()
        .get(&library)
        .and_then(Weak::upgrade)
        .ok_or_else(|| anyhow!("unknown library"))?;
    let client_vv = decode_vv(&vv)?;
    write_msg(
        send,
        &Msg::Reply {
            updates: shared.updates_since_in(library, &client_vv)?,
            vv: shared.oplog_vv_in(library)?.encode(),
        },
    )
    .await?;
    let Msg::Updates(bytes) = read_msg(recv).await? else {
        bail!("expected updates");
    };
    shared.import_bytes_in(library, &bytes)?;
    write_msg(
        send,
        &Msg::Done {
            vv: shared.oplog_vv_in(library)?.encode(),
        },
    )
    .await?;
    Ok(())
}
