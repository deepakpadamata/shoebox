//! The `shoebox/loro/0` sync exchange (one bidirectional stream):
//!
//! 1. client → `Hello{library, vv}`   (its version vector)
//! 2. server → `Reply{updates, vv}`   (what the client lacks, and the server's own vector)
//! 3. client → `Updates(bytes)`       (what the server lacks)
//! 4. server → `Done{vv}`             (the server's vector after importing)
//!
//! The client compares that last vector with its own: equal means both replicas hold exactly the
//! same operations. If either side wrote in the meantime the client runs another round.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use shoebox_core::store::LibraryId;

const MAX_MESSAGE: usize = 1 << 30;

#[derive(Serialize, Deserialize)]
pub(crate) enum Msg {
    Hello { library: LibraryId, vv: Vec<u8> },
    Reply { updates: Vec<u8>, vv: Vec<u8> },
    Updates(Vec<u8>),
    Done { vv: Vec<u8> },
    Error(String),
}

pub(crate) async fn write_msg(send: &mut iroh::endpoint::SendStream, msg: &Msg) -> Result<()> {
    let body = postcard::to_stdvec(msg)?;
    ensure!(body.len() <= MAX_MESSAGE, "message too large");
    send.write_all(&(body.len() as u32).to_be_bytes()).await?;
    send.write_all(&body).await?;
    Ok(())
}

pub(crate) async fn read_msg(recv: &mut iroh::endpoint::RecvStream) -> Result<Msg> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.context("reading message")?;
    let len = u32::from_be_bytes(len) as usize;
    ensure!(len <= MAX_MESSAGE, "message too large");
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .context("reading message")?;
    match postcard::from_bytes(&body)? {
        Msg::Error(e) => bail!("peer: {e}"),
        m => Ok(m),
    }
}
