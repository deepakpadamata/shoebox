//! M2-02: the recovery kit round-trips the root key and refuses wrong phrases.

use shoebox_core::identity::KeyPair;
use shoebox_core::recovery::{RecoveryError, RecoveryKit};
use shoebox_core::store::LibraryId;

const LIB: LibraryId = LibraryId([0xAB; 16]);

fn kit() -> RecoveryKit {
    RecoveryKit::new(LIB, 1_700_000_000_000, KeyPair::from_seed([5; 32]))
}

#[test]
fn the_phrase_is_24_words_and_restores_the_same_key() {
    let k = kit();
    let phrase = k.phrase();
    assert_eq!(phrase.split_whitespace().count(), 24);
    let restored = RecoveryKit::key_from_phrase(&phrase, &k.root().root_public()).unwrap();
    assert_eq!(restored.public_bytes(), k.root().public_bytes());
    // Case and extra whitespace do not matter when typing a phrase in.
    let sloppy = format!("  {}  ", phrase.to_uppercase().replace(' ', "   "));
    assert!(RecoveryKit::key_from_phrase(&sloppy, &k.root().root_public()).is_ok());
}

#[test]
fn a_known_seed_gives_a_known_phrase() {
    // 32 zero bytes is the standard all-"abandon … art" BIP39 vector.
    let k = RecoveryKit::new(LIB, 0, KeyPair::from_seed([0; 32]));
    let words: Vec<&str> = k
        .phrase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .leak()
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert!(words[..23].iter().all(|w| *w == "abandon"));
    assert_eq!(words[23], "art");
}

#[test]
fn wrong_phrases_are_rejected_with_a_clear_error() {
    let k = kit();
    let root = k.root().root_public();
    // Not words from the list / wrong length / bad checksum.
    assert_eq!(
        RecoveryKit::key_from_phrase("hello world", &root).unwrap_err(),
        RecoveryError::BadPhrase
    );
    assert_eq!(
        RecoveryKit::key_from_phrase("", &root).unwrap_err(),
        RecoveryError::BadPhrase
    );
    let mut words: Vec<String> = k.phrase().split_whitespace().map(String::from).collect();
    words.swap(0, 1);
    let swapped = words.join(" ");
    let err = RecoveryKit::key_from_phrase(&swapped, &root).unwrap_err();
    assert!(
        matches!(err, RecoveryError::BadPhrase | RecoveryError::WrongLibrary),
        "{err:?}"
    );
    // A valid phrase for a different library's key.
    let other = RecoveryKit::new(LIB, 0, KeyPair::from_seed([6; 32]));
    assert_eq!(
        RecoveryKit::key_from_phrase(&other.phrase(), &root).unwrap_err(),
        RecoveryError::WrongLibrary
    );
}

#[test]
fn the_printable_text_round_trips_and_is_self_describing() {
    let k = kit();
    let text = k.to_text();
    assert!(text.contains(&hex::encode(LIB.0)), "library id");
    assert!(text.contains("2023-11-14"), "creation date: {text}");
    assert!(text.contains("Keep this somewhere safe"), "{text}");
    for (i, w) in k.phrase().split_whitespace().enumerate() {
        assert!(text.contains(&format!("{:>2}. {w}", i + 1)), "word {i}");
    }
    let parsed = RecoveryKit::from_text(&text).unwrap();
    assert_eq!(parsed.library(), LIB);
    assert_eq!(parsed.created_ms(), 1_700_000_000_000);
    assert_eq!(parsed.root().public_bytes(), k.root().public_bytes());
    assert_eq!(
        RecoveryKit::from_text("nonsense").unwrap_err(),
        RecoveryError::BadPhrase
    );
}
