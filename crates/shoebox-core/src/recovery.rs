//! The recovery kit (M2-02): the library root key as a 24-word BIP39 phrase plus a printable
//! text file. It is the only way back into a library when every device is lost.

use crate::identity::KeyPair;
use crate::store::{LibraryId, RootPublicKey};
use bip39::Mnemonic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    #[error("that is not a valid 24-word recovery phrase")]
    BadPhrase,
    #[error("that is not this library's current recovery phrase (it belongs to another library, or it was replaced)")]
    WrongLibrary,
}

#[derive(Debug, Clone)]
pub struct RecoveryKit {
    library: LibraryId,
    created_ms: u64,
    root: KeyPair,
}

impl RecoveryKit {
    pub fn new(library: LibraryId, created_ms: u64, root: KeyPair) -> Self {
        Self {
            library,
            created_ms,
            root,
        }
    }

    pub fn library(&self) -> LibraryId {
        self.library
    }
    pub fn created_ms(&self) -> u64 {
        self.created_ms
    }
    pub fn root(&self) -> &KeyPair {
        &self.root
    }

    /// The 24 words, space separated.
    pub fn phrase(&self) -> String {
        Mnemonic::from_entropy(&self.root.seed())
            .expect("32 bytes is valid BIP39 entropy")
            .to_string()
    }

    /// Recover the root key from a typed phrase and check that it is the key of the library
    /// whose root public key is `expected`. Case and spacing are forgiven.
    pub fn key_from_phrase(
        phrase: &str,
        expected: &RootPublicKey,
    ) -> Result<KeyPair, RecoveryError> {
        let key = Self::root_from_phrase(phrase)?;
        if key.root_public() != *expected {
            return Err(RecoveryError::WrongLibrary);
        }
        Ok(key)
    }

    /// The root key a phrase encodes, without knowing which library it belongs to (used when
    /// recovering onto a device that has nothing yet).
    pub fn root_from_phrase(phrase: &str) -> Result<KeyPair, RecoveryError> {
        let normalised = phrase
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mnemonic = Mnemonic::parse(&normalised).map_err(|_| RecoveryError::BadPhrase)?;
        if mnemonic.word_count() != 24 {
            return Err(RecoveryError::BadPhrase);
        }
        let seed: [u8; 32] = mnemonic
            .to_entropy()
            .try_into()
            .map_err(|_| RecoveryError::BadPhrase)?;
        Ok(KeyPair::from_seed(seed))
    }

    /// A printable text file: what it is, which library, when, and the numbered words.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("SHOEBOX RECOVERY KIT\n====================\n\n");
        out.push_str("Anyone with these 24 words can take over this photo library.\n");
        out.push_str(
            "Keep this somewhere safe (print it, don't store it next to your photos).\n\n",
        );
        out.push_str(&format!("Library:  {}\n", hex::encode(self.library.0)));
        out.push_str(&format!("Created:  {}\n", iso_date(self.created_ms)));
        out.push_str(&format!("Created-ms: {}\n\n", self.created_ms));
        for (i, w) in self.phrase().split_whitespace().enumerate() {
            out.push_str(&format!("{:>2}. {w}\n", i + 1));
        }
        out
    }

    /// Parse [`RecoveryKit::to_text`] output (the words and library id are what matters).
    pub fn from_text(text: &str) -> Result<Self, RecoveryError> {
        let field = |name: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(name))
                .map(|v| v.trim().to_string())
        };
        let library: [u8; 16] = field("Library:")
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| b.try_into().ok())
            .ok_or(RecoveryError::BadPhrase)?;
        let created_ms = field("Created-ms:")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let words: Vec<&str> = text
            .lines()
            .filter_map(|l| {
                let (n, w) = l.trim().split_once(". ")?;
                n.trim().parse::<u32>().ok()?;
                Some(w.trim())
            })
            .collect();
        let mnemonic = Mnemonic::parse(words.join(" ").to_lowercase())
            .map_err(|_| RecoveryError::BadPhrase)?;
        let seed: [u8; 32] = mnemonic
            .to_entropy()
            .try_into()
            .map_err(|_| RecoveryError::BadPhrase)?;
        Ok(Self::new(
            LibraryId(library),
            created_ms,
            KeyPair::from_seed(seed),
        ))
    }
}

/// `YYYY-MM-DD` (UTC) from epoch milliseconds, without a date library.
fn iso_date(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
