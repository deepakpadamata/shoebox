//! Merge rules shared by every backend (M0-05). Skeleton: behaviour is added test-first.

use crate::hlc::Hlc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Last-writer-wins register ordered by [`Hlc`] (device id breaks ties).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Lww<T> {
    pub value: T,
    pub hlc: Hlc,
}

/// Add-wins observed-remove set. An element is present iff some add tag has not been observed
/// (and removed) by a remove.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OrSet<T: Ord> {
    pub adds: BTreeMap<T, BTreeSet<Hlc>>,
    pub removes: BTreeMap<T, BTreeSet<Hlc>>,
}
