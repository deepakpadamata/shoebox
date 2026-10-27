//! Multi-node test harness (M1-02).

mod assertions;
mod cluster;

pub use assertions::{assert_converged, assert_holders};
pub use cluster::{time_scale, wait_until, Cluster};
