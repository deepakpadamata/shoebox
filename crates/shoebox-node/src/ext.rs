//! Extension protocols: a backend crate that depends on this one (the Loro library store)
//! plugs a handler in for its ALPN after the node has started, because iroh's router is fixed
//! when the node opens.

use anyhow::Result;
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use std::fmt;
use std::sync::{Arc, Mutex};

/// Handles incoming connections of one extension ALPN.
pub trait ConnHandler: Send + Sync {
    fn handle(&self, conn: Connection) -> BoxFuture<'static, Result<()>>;
}

pub type ExtSlot = Arc<Mutex<Option<Arc<dyn ConnHandler>>>>;

#[derive(Clone)]
pub(crate) struct ExtProtocol(pub ExtSlot);

impl fmt::Debug for ExtProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExtProtocol")
    }
}

impl ProtocolHandler for ExtProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let handler = self.0.lock().unwrap().clone();
        match handler {
            Some(h) => h
                .handle(conn)
                .await
                .map_err(|e| AcceptError::from_err(std::io::Error::other(format!("{e:#}")))),
            None => {
                conn.close(2u32.into(), b"not available");
                Ok(())
            }
        }
    }
}
