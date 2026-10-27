//! In-process clusters of real nodes with scripted partitions.

use anyhow::{bail, Result};
use shoebox_core::clock::ManualClock;
use shoebox_core::model::Role;
use shoebox_node::{Node, NodeConfig};
use std::collections::HashSet;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `n` nodes in temp dirs, local-only (no relay, no discovery service), that already know each
/// other's addresses. All share one fake clock for HLC-dependent tests.
pub struct Cluster {
    nodes: Vec<Node>,
    clock: ManualClock,
    partitions: Mutex<HashSet<(usize, usize)>>,
    offline: Mutex<HashSet<usize>>,
    _dir: tempfile::TempDir,
}

impl Cluster {
    pub async fn new(n: usize) -> Result<Cluster> {
        Self::with_roles(&vec![Role::Keeper; n]).await
    }

    /// One node per role, in order (node `i` gets `roles[i]`).
    pub async fn with_roles(roles: &[Role]) -> Result<Cluster> {
        let dir = tempfile::tempdir()?;
        let mut nodes = Vec::with_capacity(roles.len());
        for (i, role) in roles.iter().enumerate() {
            let mut cfg =
                NodeConfig::local_only(dir.path().join(format!("node-{i}")), format!("node-{i}"))
                    // Tests need blob GC to happen within a test's lifetime.
                    .with_gc_interval(std::time::Duration::from_millis(200))
                    // ...but not while a node is busy (imports, fetches, syncs): see `Activity`.
                    .with_gc_quiet(std::time::Duration::from_millis(400));
            cfg.role = *role;
            nodes.push(Node::open(cfg).await?);
        }
        for a in &nodes {
            for b in &nodes {
                if a.id() != b.id() {
                    a.add_peer_addr(b.addr());
                }
            }
        }
        Ok(Cluster {
            nodes,
            clock: ManualClock::new(1_000_000),
            partitions: Default::default(),
            offline: Default::default(),
            _dir: dir,
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, i: usize) -> &Node {
        &self.nodes[i]
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn clock(&self) -> &ManualClock {
        &self.clock
    }

    /// Cut all traffic between nodes `a` and `b` (existing connections are dropped).
    pub fn partition(&self, a: usize, b: usize) {
        self.partitions.lock().unwrap().insert((a.min(b), a.max(b)));
        self.apply();
    }

    /// Remove every partition (offline nodes stay offline).
    pub fn heal(&self) {
        self.partitions.lock().unwrap().clear();
        self.apply();
    }

    /// Cut node `i` off from every other node.
    pub fn take_offline(&self, i: usize) {
        self.offline.lock().unwrap().insert(i);
        self.apply();
    }

    pub fn bring_online(&self, i: usize) {
        self.offline.lock().unwrap().remove(&i);
        self.apply();
    }

    /// Recompute which ordered pairs are blocked from the partition and offline sets.
    fn apply(&self) {
        let partitions = self.partitions.lock().unwrap().clone();
        let offline = self.offline.lock().unwrap().clone();
        for i in 0..self.nodes.len() {
            for j in 0..self.nodes.len() {
                if i == j {
                    continue;
                }
                let cut = partitions.contains(&(i.min(j), i.max(j)))
                    || offline.contains(&i)
                    || offline.contains(&j);
                if cut {
                    self.nodes[i].block_peer(self.nodes[j].id());
                } else {
                    self.nodes[i].unblock_peer(self.nodes[j].id());
                }
            }
        }
    }

    pub async fn shutdown(&self) {
        for node in &self.nodes {
            let _ = node.shutdown().await;
        }
    }
}

/// How much longer to wait on slow machines: `SHOEBOX_TEST_TIME_SCALE` (CI sets 3; default 1).
/// A real stall still fails, only later.
pub fn time_scale() -> f64 {
    std::env::var("SHOEBOX_TEST_TIME_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|s| *s >= 1.0)
        .unwrap_or(1.0)
}

/// Poll `f` every 25 ms until it returns true; errors after `timeout` (times [`time_scale`]).
pub async fn wait_until<F, Fut>(timeout: Duration, mut f: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let timeout = timeout.mul_f64(time_scale());
    let deadline = Instant::now() + timeout;
    loop {
        if f().await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out after {timeout:?} waiting for condition");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
