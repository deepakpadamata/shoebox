//! The diagnostics bundle (M6-07): what a person can send to get help, with nothing in it that
//! says what is in their library. Counts, versions, device roles and short ids, the state of
//! backup and sync, and the tail of the app's own log with every file name, path, coordinate,
//! name and invitation taken out. Nothing is uploaded: the UI shows the text to copy or save.

use crate::core::AppCore;
use crate::types::DiagnosticsBundle;
use anyhow::Result;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const RING_LINES: usize = 400;
/// Lines of the log tail put in the bundle.
const TAIL_LINES: usize = 200;

fn ring() -> &'static Mutex<VecDeque<String>> {
    static RING: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Keeps the most recent log lines in memory (install next to the normal log output).
pub struct RingLayer;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > tracing::Level::INFO {
            return;
        }
        let mut m = Message(String::new());
        event.record(&mut m);
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("{when} {:<5} {} {}", meta.level(), meta.target(), m.0);
        let mut ring = ring().lock().unwrap();
        if ring.len() >= RING_LINES {
            ring.pop_front();
        }
        ring.push_back(line);
    }
}

/// Log to stderr as usual and keep the recent lines for the diagnostics bundle.
pub fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,loro_internal=warn".into());
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(RingLayer)
        .try_init();
}

const FILE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "avif", "gif", "webp", "tif", "tiff", "dng", "cr2",
    "cr3", "nef", "arw", "raf", "orf", "rw2", "mov", "mp4", "m4v", "avi", "mkv", "3gp", "pdf",
    "txt", "json", "zip",
];

fn is_coordinate(tok: &str) -> bool {
    let t = tok.trim_start_matches(['-', '+']);
    match t.split_once('.') {
        Some((int, frac)) => {
            !int.is_empty()
                && int.len() <= 3
                && frac.len() >= 4
                && int.bytes().all(|b| b.is_ascii_digit())
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

fn redact_token(tok: &str) -> String {
    let lower = tok.to_ascii_lowercase();
    if lower.starts_with("shoebox-")
        || lower.contains("shoebox-pair")
        || lower.contains("shoebox-album")
    {
        return "<ticket>".into();
    }
    if tok.contains('@') {
        return "<email>".into();
    }
    // A path: several segments, a home shortcut, or a drive letter.
    let segments = tok.split(['/', '\\']).filter(|s| !s.is_empty()).count();
    let drive =
        tok.len() > 2 && tok.as_bytes()[1] == b':' && matches!(tok.as_bytes()[2], b'\\' | b'/');
    if (tok.contains('/') || tok.contains('\\'))
        && (segments >= 2 || tok.starts_with('/') || tok.starts_with('~'))
        || drive
        || tok.starts_with('~')
    {
        // Keep URLs' scheme out of it too: nothing in a log should name a place on disk.
        return "<path>".into();
    }
    if let Some((_, ext)) = tok.rsplit_once('.') {
        if FILE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            return "<file>".into();
        }
    }
    if is_coordinate(tok) {
        return "<coordinate>".into();
    }
    if tok.len() >= 32 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
        return format!("{}…", &tok[..8]);
    }
    tok.to_string()
}

/// Take everything identifying out of one log line. `names` are the names this person gave to
/// devices, albums and so on (they vary, so they are passed in).
pub fn redact(line: &str, names: &[String]) -> String {
    // Names first (they may contain spaces or look like anything).
    let mut text = line.to_string();
    for n in names.iter().filter(|n| n.trim().chars().count() >= 2) {
        let (hay, needle) = (text.to_lowercase(), n.trim().to_lowercase());
        let mut out = String::new();
        let mut last = 0;
        // Lowercasing can change byte lengths for exotic characters: only replace when the
        // lengths agree, otherwise fall back to dropping the whole line's text after tokens.
        if hay.len() == text.len() {
            while let Some(p) = hay[last..].find(&needle) {
                out.push_str(&text[last..last + p]);
                out.push_str("<name>");
                last += p + needle.len();
            }
            out.push_str(&text[last..]);
            text = out;
        }
    }
    // Then token by token, keeping the punctuation that separates tokens.
    let mut out = String::new();
    let mut tok = String::new();
    let is_sep = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '=' | '<' | '>'
            )
    };
    for c in text.chars() {
        if is_sep(c) {
            if !tok.is_empty() {
                out.push_str(&redact_token(tok.trim_end_matches([':', '.'])));
                if tok.ends_with(':') {
                    out.push(':');
                }
                tok.clear();
            }
            out.push(c);
        } else {
            tok.push(c);
        }
    }
    if !tok.is_empty() {
        out.push_str(&redact_token(tok.trim_end_matches([':', '.'])));
    }
    out
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

impl AppCore {
    /// The text to send when asking for help.
    pub async fn diagnostics(&self) -> Result<DiagnosticsBundle> {
        let mut names: Vec<String> = vec![self.device_name()];
        let mut out = String::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(out, "Shoebox diagnostics");
        let _ = writeln!(
            out,
            "No photos, names, places or file names are in this report.\n"
        );
        let _ = writeln!(out, "generated (unix seconds): {now}");
        let _ = writeln!(
            out,
            "app: {} on {} {}, data layout {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            shoebox_node::format::CURRENT_FORMAT
        );
        let state = self.app_state().await?;
        let _ = writeln!(
            out,
            "device: {} role={} membership={} recovery_kit_saved={}",
            short(&state.device_id),
            state.role,
            state.membership,
            state.recovery_kit_saved
        );
        if state.has_library {
            let c = self.counts().await?;
            let _ = writeln!(
                out,
                "library: {} photos, {} favorites, {} deleted, {} at risk (target {} copies)",
                c.photos, c.favorites, c.deleted, c.at_risk, c.redundancy_target
            );
            let _ = writeln!(out, "\ndevices:");
            for d in self.devices().await? {
                names.push(d.name.clone());
                let _ = writeln!(
                    out,
                    "  {} role={} online={} path={} latency_ms={} admin={} revoked={}{}",
                    short(&d.id),
                    d.role,
                    d.online,
                    d.path,
                    d.latency_ms
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "-".into()),
                    d.admin,
                    d.revoked,
                    if d.is_this_device {
                        " (this device)"
                    } else {
                        ""
                    }
                );
            }
            for a in self.albums().await? {
                names.push(a.name);
            }
            let b = self.backup_status().await?;
            let _ = writeln!(
                out,
                "\nbackup: enabled={} permission={} imported={} pending={} paused={}",
                b.policy.enabled,
                b.permission,
                b.imported,
                b.pending,
                b.paused.as_deref().unwrap_or("no")
            );
            if let Some(e) = &b.last_error {
                let _ = writeln!(out, "backup last error: {}", redact(e, &names));
            }
            let shared = self.shared_albums().await?;
            let _ = writeln!(out, "\nshared albums: {}", shared.len());
            for s in &shared {
                names.push(s.name.clone());
                let _ = writeln!(
                    out,
                    "  {} role={} photos={} people={} keep_originals={}",
                    short(&s.id),
                    s.role,
                    s.item_count,
                    s.member_count,
                    s.keep_originals
                );
                for m in self.shared_members(&s.id).await.unwrap_or_default() {
                    names.push(m.name);
                }
            }
        }
        let tail: Vec<String> = ring().lock().unwrap().iter().cloned().collect();
        let _ = writeln!(out, "\nrecent log (redacted):");
        for line in tail.iter().rev().take(TAIL_LINES).rev() {
            let _ = writeln!(out, "  {}", redact(line, &names));
        }
        Ok(DiagnosticsBundle {
            file_name: format!("shoebox-diagnostics-{now}.txt"),
            text: out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["Ana's laptop".into(), "Lisbon trip".into(), "Ben".into()]
    }

    #[test]
    fn paths_files_coordinates_names_and_tickets_are_removed() {
        let cases = [
            (
                "imported /Users/ana/Pictures/Lisbon/IMG_0042.jpg ok",
                "imported <path> ok",
            ),
            (
                "failed to read C:\\Users\\Ana\\Pictures\\a.heic: denied",
                "failed to read <path>: denied",
            ),
            (
                "skipping IMG_0042.JPG (unsupported)",
                "skipping <file> (unsupported)",
            ),
            (
                "exif gps=38.7223,-9.1393 found",
                "exif gps=<coordinate>,<coordinate> found",
            ),
            (
                "lat 38.722300 lon -9.139300",
                "lat <coordinate> lon <coordinate>",
            ),
            (
                "device Ana's laptop joined album Lisbon trip",
                "device <name> joined album <name>",
            ),
            (
                "invite SHOEBOX-ALBUM1-BBM3QE3R7FMA3 used",
                "invite <ticket> used",
            ),
            ("pair shoebox-pair1abcdef", "pair <ticket>"),
            ("mail ana@example.org sent", "mail <email> sent"),
            (
                "peer 316ed71554316ed71554316ed71554316ed71554316ed71554316ed715543210 up",
                "peer 316ed715… up",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input, &names()), want, "from `{input}`");
        }
    }

    #[test]
    fn ordinary_diagnostic_text_survives() {
        let line = "sync with 316ed715 finished in 0.0123s (3 changes, version 1.2.3)";
        assert_eq!(redact(line, &names()), line);
        assert_eq!(
            redact("2 photos at risk, retry in 10s", &names()),
            "2 photos at risk, retry in 10s"
        );
    }

    #[test]
    fn a_hostile_log_line_comes_out_clean() {
        let line = r#"error: open("/home/ben/photos/2024/birthday party.mov") at 51.501364, -0.141890 for Ben <ben@x.io> album=Lisbon trip token=shoebox-pair1deadbeef"#;
        let clean = redact(line, &names());
        for leak in [
            "/home", "birthday", ".mov", "51.50", "0.1418", "Ben", "ben@", "Lisbon", "deadbeef",
        ] {
            assert!(!clean.contains(leak), "`{leak}` leaked: {clean}");
        }
    }
}
