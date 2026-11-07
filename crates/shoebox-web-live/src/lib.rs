//! The browser's end of `shoebox/webview/0` (W1-08). A browser has no UDP, so it reaches a serving device through
//! the relay by the device's endpoint id alone, then sends one small request per stream. The wire format is
//! documented in `crates/shoebox-node/src/web_live.rs`. The page decrypts and checks what comes back with
//! `shoebox-web`, exactly as for a Cloud link.
#![cfg(target_arch = "wasm32")]

use iroh::address_lookup::{
    AddressLookup, AddressLookupBuilder, AddressLookupBuilderError, EndpointInfo,
    Error as LookupError, Item, PkarrResolver,
};
use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl};
use n0_future::boxed::BoxStream;
use n0_future::time::{timeout, Duration};
use n0_future::StreamExt;
use wasm_bindgen::prelude::*;

const ALPN: &[u8] = b"shoebox/webview/0";
const CONNECT: Duration = Duration::from_secs(25);
const REPLY: Duration = Duration::from_secs(60);

/// A connection to one of a link's serving devices.
#[wasm_bindgen]
pub struct Live {
    _endpoint: Endpoint,
    conn: Connection,
}

fn err(kind: &str) -> JsError {
    JsError::new(kind)
}

#[wasm_bindgen]
impl Live {
    /// Try each serving device in turn (`servers` is their 32-byte ids one after another); the first that answers
    /// wins. Fails with `unreachable` when none does.
    pub async fn connect(servers: &[u8]) -> Result<Live, JsError> {
        // n0's defaults, except that relay hosts lose the trailing dot iroh writes (`….iroh.link.`): Firefox drops a
        // Content-Security-Policy source ending in a dot, so the page can allow only `*.iroh.link`.
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Custom(RelayMap::from_iter(
                iroh::defaults::prod::default_relay_map()
                    .relays::<Vec<_>>()
                    .into_iter()
                    .map(|r| RelayConfig::from(undotted(&r.url))),
            )))
            .address_lookup(UndottedBuilder(PkarrResolver::n0_dns()))
            .bind()
            .await
            .map_err(|_| err("unreachable"))?;
        for id in servers.chunks_exact(32) {
            let Ok(id) = EndpointId::from_bytes(id.try_into().expect("32 bytes")) else {
                continue;
            };
            if let Ok(Ok(conn)) =
                timeout(CONNECT, endpoint.connect(EndpointAddr::new(id), ALPN)).await
            {
                return Ok(Live {
                    _endpoint: endpoint,
                    conn,
                });
            }
        }
        Err(err("unreachable"))
    }

    /// The sealed manifest.
    pub async fn manifest(&self, share_id: &[u8]) -> Result<Vec<u8>, JsError> {
        let mut req = vec![1u8];
        req.extend_from_slice(share_id);
        Ok(self.ask(req).await?.1)
    }

    /// `len` bytes of a blob from `offset` (at most 8 MiB come back at once). The first number of the result is the
    /// whole blob's size, followed by the bytes.
    pub async fn blob_range(
        &self,
        share_id: &[u8],
        cid: &[u8],
        offset: f64,
        len: u32,
    ) -> Result<Vec<u8>, JsError> {
        let mut req = vec![2u8];
        req.extend_from_slice(share_id);
        req.extend_from_slice(cid);
        req.extend_from_slice(&(offset as u64).to_le_bytes());
        req.extend_from_slice(&len.to_le_bytes());
        let (total, bytes) = self.ask(req).await?;
        let mut out = total.to_le_bytes().to_vec();
        out.extend_from_slice(&bytes);
        Ok(out)
    }

    pub fn close(&self) {
        self.conn.close(0u32.into(), b"done");
    }
}

/// `url` without a trailing dot on its host name.
fn undotted(url: &RelayUrl) -> RelayUrl {
    match url.host_str() {
        Some(host) if host.ends_with('.') && host.len() > 1 => {
            let mut plain = (**url).clone();
            if plain.set_host(Some(&host[..host.len() - 1])).is_ok() {
                return plain.into();
            }
            url.clone()
        }
        _ => url.clone(),
    }
}

/// An address lookup whose answers name relays without the trailing dot (a serving device publishes its home relay
/// as iroh writes it).
#[derive(Debug)]
struct Undotted<T>(T);

#[derive(Debug)]
struct UndottedBuilder<T>(T);

impl<B: AddressLookupBuilder> AddressLookupBuilder for UndottedBuilder<B> {
    fn into_address_lookup(
        self,
        endpoint: &Endpoint,
    ) -> Result<impl AddressLookup, AddressLookupBuilderError> {
        Ok(Undotted(self.0.into_address_lookup(endpoint)?))
    }
}

impl<L: AddressLookup> AddressLookup for Undotted<L> {
    fn resolve(&self, id: EndpointId) -> Option<BoxStream<Result<Item, LookupError>>> {
        let stream = self.0.resolve(id)?;
        Some(Box::pin(stream.map(|item| {
            item.map(|item| {
                let info = item.endpoint_info();
                let mut data = info.data.clone();
                let relays: Vec<RelayUrl> = data.relay_urls().map(undotted).collect();
                data.clear_relay_urls();
                for url in relays {
                    data.add_relay_url(url);
                }
                Item::new(
                    EndpointInfo::from_parts(info.endpoint_id, data),
                    item.provenance(),
                    item.last_updated(),
                )
            })
        })))
    }
}

impl Live {
    async fn ask(&self, req: Vec<u8>) -> Result<(u64, Vec<u8>), JsError> {
        let run = async {
            let (mut send, mut recv) = self.conn.open_bi().await.ok()?;
            send.write_all(&req).await.ok()?;
            send.finish().ok()?;
            recv.read_to_end(32 * 1024 * 1024).await.ok()
        };
        let reply = timeout(REPLY, run)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| err("unreachable"))?;
        match reply.split_first() {
            Some((0, rest)) if rest.len() >= 8 => Ok((
                u64::from_le_bytes(rest[..8].try_into().expect("8 bytes")),
                rest[8..].to_vec(),
            )),
            Some((1, _)) => Err(err("gone")),
            Some((2, _)) => Err(err("expired")),
            Some((3, _)) => Err(err("busy")),
            _ => Err(err("unreachable")),
        }
    }
}
