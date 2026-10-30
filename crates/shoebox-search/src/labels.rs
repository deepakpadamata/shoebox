//! Labels (S1-13): everyday words for what a photo shows ("dog", "beach", "receipt"), picked by
//! comparing its picture embedding with the text embedding of each label. They make content
//! searchable as words: typos and prefixes, suggestions while typing ("Things"), and a device
//! without a usable text model still finds "dog".
//!
//! The list is `data/labels.txt`; its embeddings (`data/labels.bin`, prompt "a photo of a
//! {label}") are made with the shipped text model by `cargo xtask models`. Text shards record
//! label numbers, so the list has a version ([`LABELS_VERSION`], in the shard's `dims` field):
//! a shard of another version is re-labelled from its photos' vectors (S1-15), never trusted.

use crate::shard::Vec8;
use crate::text::fold;
use shoebox_core::model::ModelId;
use std::collections::HashMap;

/// Bump when `labels.txt` changes (a test pins the file's hash to this version).
pub const LABELS_VERSION: u16 = 1;
/// The prompt each label is embedded in.
pub const PROMPT: &str = "a photo of a {}";
/// CLIP's logit scale: similarities × 100 before the softmax over labels.
pub const LOGIT_SCALE: f32 = 100.0;
/// At most this many labels per photo.
pub const MAX_LABELS: usize = 8;
/// A label is kept when its share of the softmax is at least this (tuned on the eval set,
/// `docs/notes/search-models.md`).
pub const MIN_PROBABILITY: f32 = 0.10;

const MAGIC: &[u8; 4] = b"SBLB";

pub struct Labels {
    names: Vec<String>,
    /// Folded label or synonym → label.
    terms: HashMap<String, u16>,
    /// Synonyms per label, as written.
    synonyms: Vec<Vec<String>>,
    model: Option<ModelId>,
    dims: usize,
    vectors: Vec<Vec8>,
}

/// The list as written: `(label, synonyms)` per line.
pub fn parse_list(text: &str) -> Vec<(String, Vec<String>)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(':') {
            Some((name, syn)) => (
                name.trim().to_string(),
                syn.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            ),
            None => (l.to_string(), vec![]),
        })
        .collect()
}

/// `labels.bin`: the label vectors for one model.
pub fn encode_vectors(model: ModelId, version: u16, vectors: &[Vec8], dims: u16) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.push(1);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&model.0);
    out.extend_from_slice(&(vectors.len() as u32).to_le_bytes());
    out.extend_from_slice(&dims.to_le_bytes());
    for v in vectors {
        out.extend_from_slice(&v.scale.to_le_bytes());
        let mut q = v.v.clone();
        q.resize(usize::from(dims), 0);
        out.extend(q.iter().map(|b| *b as u8));
    }
    out
}

fn decode_vectors(bytes: &[u8]) -> Option<(ModelId, u16, usize, Vec<Vec8>)> {
    if bytes.len() < 45 || &bytes[..4] != MAGIC || bytes[4] != 1 {
        return None;
    }
    let version = u16::from_le_bytes(bytes[5..7].try_into().ok()?);
    let model = ModelId(bytes[7..39].try_into().ok()?);
    let n = u32::from_le_bytes(bytes[39..43].try_into().ok()?) as usize;
    let dims = usize::from(u16::from_le_bytes(bytes[43..45].try_into().ok()?));
    let body = &bytes[45..];
    if body.len() != n * (4 + dims) {
        return None;
    }
    let vectors = body
        .chunks_exact(4 + dims)
        .map(|c| Vec8 {
            scale: f32::from_le_bytes(c[..4].try_into().expect("4")),
            v: c[4..].iter().map(|b| *b as i8).collect(),
        })
        .collect();
    Some((model, version, dims, vectors))
}

impl std::fmt::Debug for Labels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Labels")
            .field("len", &self.names.len())
            .field("model", &self.model)
            .finish()
    }
}

impl Labels {
    /// The list and vectors compiled into this build. Without vectors for the current model
    /// (a development checkout before `cargo xtask models`) labels can be looked up but not
    /// assigned.
    pub fn bundled() -> Self {
        Self::new(
            include_str!("../data/labels.txt"),
            include_bytes!("../data/labels.bin"),
        )
    }

    /// [`Labels::bundled`], parsed once per process.
    pub fn shared() -> std::sync::Arc<Labels> {
        static ONCE: std::sync::OnceLock<std::sync::Arc<Labels>> = std::sync::OnceLock::new();
        ONCE.get_or_init(|| std::sync::Arc::new(Self::bundled()))
            .clone()
    }

    pub fn new(list: &str, vectors: &[u8]) -> Self {
        let parsed = parse_list(list);
        let mut terms = HashMap::new();
        let mut names = vec![];
        let mut synonyms = vec![];
        for (i, (name, syn)) in parsed.into_iter().enumerate() {
            let i = i as u16;
            terms.entry(fold(&name)).or_insert(i);
            for s in &syn {
                terms.entry(fold(s)).or_insert(i);
            }
            names.push(name);
            synonyms.push(syn);
        }
        let (model, dims, vectors) = match decode_vectors(vectors) {
            Some((model, version, dims, v))
                if version == LABELS_VERSION && v.len() == names.len() =>
            {
                (Some(model), dims, v)
            }
            _ => (None, 0, vec![]),
        };
        Self {
            names,
            terms,
            synonyms,
            model,
            dims,
            vectors,
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn name(&self, i: u16) -> Option<&str> {
        self.names.get(usize::from(i)).map(String::as_str)
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn synonyms(&self, i: u16) -> &[String] {
        self.synonyms
            .get(usize::from(i))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The label a word or phrase means ("puppy" → "dog").
    pub fn lookup(&self, term: &str) -> Option<u16> {
        self.terms.get(&fold(term)).copied()
    }

    /// The model the label vectors were made with, if this build has them.
    pub fn model(&self) -> Option<ModelId> {
        self.model
    }

    /// Labels for a photo's (normalised) embedding of the same model: the softmax over every
    /// label's similarity (× [`LOGIT_SCALE`]), at most [`MAX_LABELS`] with a share of at least
    /// `min_p`, best first, with the share in 1/255.
    pub fn assign(&self, v: &[f32], min_p: f32) -> Vec<(u16, u8)> {
        if self.vectors.is_empty() || v.len() != self.dims {
            return vec![];
        }
        let logits: Vec<f32> = self
            .vectors
            .iter()
            .map(|l| {
                let d: f32 = l.v.iter().zip(v).map(|(a, b)| f32::from(*a) * b).sum();
                d * l.scale * LOGIT_SCALE
            })
            .collect();
        let max = logits.iter().copied().fold(f32::MIN, f32::max);
        let exp: Vec<f32> = logits.iter().map(|x| (x - max).exp()).collect();
        let sum: f32 = exp.iter().sum();
        let mut out: Vec<(u16, f32)> = exp
            .iter()
            .enumerate()
            .map(|(i, e)| (i as u16, e / sum))
            .filter(|(_, p)| *p >= min_p)
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(MAX_LABELS);
        out.into_iter()
            .map(|(i, p)| (i, (p * 255.0).round().clamp(1.0, 255.0) as u8))
            .collect()
    }

    /// Words for the facts index: each label with its synonyms, so "puppy" finds a dog.
    pub fn words(&self, labels: &[(u16, u8)]) -> String {
        let mut out = vec![];
        for (i, _) in labels {
            if let Some(n) = self.name(*i) {
                out.push(n.to_string());
                out.extend(self.synonyms(*i).iter().cloned());
            }
        }
        out.join("\n")
    }
}
