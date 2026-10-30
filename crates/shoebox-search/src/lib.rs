//! Shoebox search: everything a search does at the moment it runs, on every device (epic S1).
//!
//! Like `shoebox-core`, this crate has **no async runtime, no iroh and no ML runtime**: models sit
//! behind traits that the node and apps implement, so a Pi links the same parser and indexes as a
//! desktop.

pub mod calendar;
pub mod clip_tokenizer;
pub mod encoder;
pub mod faces;
pub mod facts;
pub mod labels;
pub mod lang;
pub mod models;
pub mod ocr;
pub mod people;
pub mod places;
pub mod query;
pub mod shard;
pub mod text;
pub mod vectors;

pub use calendar::Zone;
pub use facts::{FactsIndex, Field, Hits, PhotoFacts};
pub use query::{
    parse, DateFilter, Filter, FilterKind, MediaKind, NameKind, ParseContext, Query, Vocabulary,
};
