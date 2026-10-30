//! The CLIP text tokenizer (byte-level BPE, 49,408 tokens), as OpenCLIP uses it: lowercase,
//! split into words, byte-level BPE with `</w>` marking word ends, then `[start] … [end]` padded
//! with zeros to 77 tokens. Built from the model's `merges.txt` alone: the vocabulary is the 256
//! byte symbols, the same with `</w>`, one token per merge, and the two special tokens.

use std::collections::HashMap;

pub const CONTEXT: usize = 77;
pub const START: i32 = 49_406;
pub const END: i32 = 49_407;

pub struct ClipTokenizer {
    encoder: HashMap<String, i32>,
    ranks: HashMap<(String, String), u32>,
    byte_char: [char; 256],
}

/// GPT-2's map from bytes to printable characters (so BPE never sees whitespace or controls),
/// indexed by byte, and the characters in vocabulary order (printable bytes first).
fn bytes_to_unicode() -> ([char; 256], Vec<char>) {
    let mut printable: Vec<u32> = (u32::from(b'!')..=u32::from(b'~'))
        .chain(0xA1..=0xAC)
        .chain(0xAE..=0xFF)
        .collect();
    let mut chars: Vec<u32> = printable.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !printable.contains(&b) {
            printable.push(b);
            chars.push(256 + n);
            n += 1;
        }
    }
    let mut out = ['\0'; 256];
    let mut order = Vec::with_capacity(256);
    for (b, c) in printable.iter().zip(&chars) {
        let c = char::from_u32(*c).expect("valid");
        out[*b as usize] = c;
        order.push(c);
    }
    (out, order)
}

#[derive(Debug, PartialEq, Eq)]
pub struct TokenizerError(pub String);

impl ClipTokenizer {
    /// The tokenizer of the bundled model (its merges are compiled in: about 0.5 MB).
    pub fn bundled() -> Self {
        Self::from_merges(include_str!("../data/clip-merges.txt")).expect("bundled merges")
    }

    /// From the text of `merges.txt` (a `#version` line, then one merge per line).
    pub fn from_merges(merges: &str) -> Result<Self, TokenizerError> {
        let (byte_char, order) = bytes_to_unicode();
        let mut vocab: Vec<String> = order.iter().map(|c| c.to_string()).collect();
        vocab.extend(order.iter().map(|c| format!("{c}</w>")));
        let mut ranks = HashMap::new();
        let lines = merges
            .lines()
            .filter(|l| !l.starts_with("#version") && !l.trim().is_empty());
        // 49,408 = 512 byte symbols + merges + 2 special tokens.
        for (i, line) in lines.take(49_408 - 512 - 2).enumerate() {
            let (a, b) = line
                .split_once(' ')
                .ok_or_else(|| TokenizerError(format!("bad merge line {line:?}")))?;
            vocab.push(format!("{a}{b}"));
            ranks.insert((a.to_string(), b.to_string()), i as u32);
        }
        if vocab.len() != 49_408 - 2 {
            return Err(TokenizerError(format!(
                "expected 48,894 merges, found {}",
                vocab.len() - 512
            )));
        }
        vocab.push("<|startoftext|>".into());
        vocab.push("<|endoftext|>".into());
        let encoder = vocab
            .into_iter()
            .enumerate()
            .map(|(i, s)| (s, i as i32))
            .collect();
        Ok(Self {
            encoder,
            ranks,
            byte_char,
        })
    }

    /// Token ids for `text`, without the start and end tokens.
    pub fn encode(&self, text: &str) -> Vec<i32> {
        let mut out = vec![];
        for word in pre_tokenize(text) {
            let symbols: String = word.bytes().map(|b| self.byte_char[b as usize]).collect();
            for piece in self.bpe(&symbols) {
                if let Some(id) = self.encoder.get(&piece) {
                    out.push(*id);
                }
            }
        }
        out
    }

    /// The model's input: `[START] tokens [END]`, cut to 77 (keeping `END` last), zero padded.
    pub fn context(&self, text: &str) -> [i32; CONTEXT] {
        let mut ids = vec![START];
        ids.extend(self.encode(text));
        ids.truncate(CONTEXT - 1);
        ids.push(END);
        let mut out = [0; CONTEXT];
        out[..ids.len()].copy_from_slice(&ids);
        out
    }

    fn bpe(&self, word: &str) -> Vec<String> {
        let chars: Vec<char> = word.chars().collect();
        let Some((last, rest)) = chars.split_last() else {
            return vec![];
        };
        let mut parts: Vec<String> = rest.iter().map(|c| c.to_string()).collect();
        parts.push(format!("{last}</w>"));
        while parts.len() > 1 {
            let best = parts
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| {
                    self.ranks
                        .get(&(w[0].clone(), w[1].clone()))
                        .map(|r| (*r, i))
                })
                .min();
            let Some((_, i)) = best else { break };
            let (a, b) = (parts[i].clone(), parts[i + 1].clone());
            // Merge every occurrence of the pair, left to right.
            let mut merged = Vec::with_capacity(parts.len());
            let mut i = 0;
            while i < parts.len() {
                if i + 1 < parts.len() && parts[i] == a && parts[i + 1] == b {
                    merged.push(format!("{a}{b}"));
                    i += 2;
                } else {
                    merged.push(parts[i].clone());
                    i += 1;
                }
            }
            parts = merged;
        }
        parts
    }
}

/// Lowercase, collapse whitespace and split as CLIP's pattern does: `'s 't 're 've 'm 'll 'd`,
/// runs of letters, single digits, and runs of anything else that is not a space.
fn pre_tokenize(text: &str) -> Vec<String> {
    let text: String = unicode_normalization::UnicodeNormalization::nfc(text)
        .collect::<String>()
        .to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let other = |c: char| !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' {
            let rest: String = chars[i + 1..chars.len().min(i + 3)].iter().collect();
            let hit = ["ll", "re", "ve"]
                .iter()
                .find(|s| rest.starts_with(*s))
                .map(|s| s.len())
                .or_else(|| {
                    ["s", "t", "m", "d"]
                        .iter()
                        .find(|s| rest.starts_with(*s))
                        .map(|_| 1)
                });
            if let Some(n) = hit {
                out.push(chars[i..=i + n].iter().collect());
                i += n + 1;
                continue;
            }
        }
        let start = i;
        if c.is_alphabetic() {
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
        } else if c.is_numeric() {
            i += 1;
        } else {
            while i < chars.len() && other(chars[i]) {
                i += 1;
            }
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}
