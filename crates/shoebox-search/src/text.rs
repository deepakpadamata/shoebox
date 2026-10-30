//! Normalising and splitting text: queries, names and indexed words go through the same steps,
//! so "Café", "cafe" and "CAFÉ" are one word.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Lower case, accents removed ("Ålesund" → "alesund").
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// A word of the input with its byte span in the original string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Folded form (see [`fold`]).
    pub word: String,
    pub start: usize,
    pub end: usize,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\''
}

/// Split `s` into words: runs of letters and digits (an apostrophe inside a word is kept, so
/// "new year's" stays two words). Each word is folded.
pub fn tokenize(s: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (is_word_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(st)) => {
                push(&mut out, s, st, i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(st) = start {
        push(&mut out, s, st, s.len());
    }
    out
}

fn push(out: &mut Vec<Token>, s: &str, start: usize, end: usize) {
    let raw = s[start..end].trim_matches('\'');
    if raw.is_empty() {
        return;
    }
    let lead = s[start..end].len() - s[start..end].trim_start_matches('\'').len();
    let start = start + lead;
    let end = start + raw.len();
    let mut word = fold(raw);
    // "new year's" → "years": possessives match the plain word.
    if let Some(stem) = word.strip_suffix("'s") {
        word = stem.to_string();
    }
    word.retain(|c| c != '\'');
    out.push(Token { word, start, end });
}

/// The folded words of `s`, for names and indexed text.
pub fn words(s: &str) -> Vec<String> {
    tokenize(s).into_iter().map(|t| t.word).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(fold("Café São Paulo ÅLESUND"), "cafe sao paulo alesund");
    }

    #[test]
    fn tokens_keep_their_spans() {
        let s = "Ana's  birthday, São Paulo";
        let t = tokenize(s);
        let w: Vec<&str> = t.iter().map(|t| t.word.as_str()).collect();
        assert_eq!(w, ["ana", "birthday", "sao", "paulo"]);
        assert_eq!(&s[t[0].start..t[0].end], "Ana's");
        assert_eq!(&s[t[2].start..t[3].end], "São Paulo");
    }

    #[test]
    fn stray_apostrophes_are_not_words() {
        assert_eq!(words("' rock 'n' roll '"), ["rock", "n", "roll"]);
    }
}
