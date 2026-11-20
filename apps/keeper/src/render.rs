//! Terminal rendering of the control-socket reports (the keeper's whole UI).

use serde_json::Value;
use std::fmt::Write;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("?")
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

pub fn status(v: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} · keeper {} · up {}",
        s(v, "name"),
        s(v, "version"),
        uptime(v["uptime_secs"].as_u64().unwrap_or(0))
    );
    if let Some(u) = v.get("update").filter(|u| u.is_object()) {
        let _ = writeln!(
            out,
            "\nA newer version is available: {} ({})",
            s(u, "version"),
            s(u, "page")
        );
    }
    if v["paired"] != true {
        out.push_str("\nNot paired with a library yet.\nOn a phone or laptop choose “Add a device”, then run:\n  shoebox-keeper pair <ticket>\n");
        return out;
    }
    let alerts = v["alerts"].as_array().cloned().unwrap_or_default();
    if !alerts.is_empty() {
        out.push_str("\nSECURITY ALERTS (clear with `shoebox-keeper alerts ack`):\n");
        for a in &alerts {
            if let Ok(a) = serde_json::from_value::<crate::alerts::Alert>(a.clone()) {
                let _ = writeln!(out, "  ! {}", crate::alerts::describe(&a));
            }
        }
    }
    let lib = &v["library"];
    let _ = writeln!(
        out,
        "\nLibrary {}  ({})",
        short(s(lib, "id")),
        s(v, "membership")
    );
    let _ = writeln!(
        out,
        "  photos           {}\n  originals held   {} of {}\n  at risk          {} (target: {} copies)",
        lib["photos"], lib["held_originals"], lib["photos"], lib["at_risk"], lib["redundancy_target"]
    );
    if lib["recovery_kit_saved"] != true {
        out.push_str("  ! the recovery kit has not been saved yet\n");
    }
    let search = &v["search"];
    if let Some(total) = search["total"].as_u64() {
        let content = search["content"].as_u64().unwrap_or(0);
        let state = if search["paused"] == true {
            "paused".to_string()
        } else if search["running"] == true {
            format!("indexing, {} photos/min", search["photos_per_min"])
        } else {
            "not indexing".to_string()
        };
        let _ = writeln!(
            out,
            "  search           {content} of {total} photos by content ({state})"
        );
    }
    if let Some(i) = v.get("import").filter(|i| i.is_object()) {
        let line = match s(i, "state") {
            "running" => import_progress(i),
            _ => "Import stopped before it finished (`shoebox-keeper import`)".into(),
        };
        let _ = writeln!(out, "  ! {line}");
    }
    let _ = writeln!(out, "\nDevices ({})", v["devices"]);
    for p in v["peers"].as_array().into_iter().flatten() {
        let latency = p["latency_ms"]
            .as_u64()
            .map(|l| format!("{l} ms"))
            .unwrap_or_else(|| "-".into());
        let _ = writeln!(
            out,
            "  {:<20} {:<7} {}",
            s(p, "name"),
            s(p, "path"),
            latency
        );
    }
    let albums = v["shared_albums"].as_array().map(Vec::len).unwrap_or(0);
    let _ = writeln!(out, "\nShared albums: {albums}");
    out
}

pub fn albums(v: &Value) -> String {
    let rows = v["albums"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return "No shared albums. Join one with `shoebox-keeper album join <invitation>`.\n"
            .into();
    }
    let mut out = String::from("ID        PHOTOS  PEOPLE  ORIGINALS  NAME\n");
    for a in rows {
        let id = a["id"].as_str().unwrap_or("");
        let _ = writeln!(
            out,
            "{:<9} {:<7} {:<7} {:<10} {}",
            &id[..id.len().min(8)],
            a["photos"].as_u64().unwrap_or(0),
            a["members"].as_u64().unwrap_or(0),
            if a["keep_originals"] == true {
                "kept"
            } else {
                "on demand"
            },
            a["name"].as_str().unwrap_or("")
        );
    }
    out
}

pub fn devices(v: &Value) -> String {
    let mut out = String::from("ID        NAME                 ROLE     STATE\n");
    for d in v["devices"].as_array().into_iter().flatten() {
        let state = if d["revoked"] == true {
            "revoked"
        } else if d["online"] == true {
            "online"
        } else {
            "offline"
        };
        let mut tags = String::new();
        if d["admin"] == true {
            tags.push_str(" admin");
        }
        if d["self"] == true {
            tags.push_str(" (this device)");
        }
        let _ = writeln!(
            out,
            "{:<9} {:<20} {:<8} {}{}",
            short(s(d, "id")),
            s(d, "name"),
            s(d, "role"),
            state,
            tags
        );
    }
    out
}

/// `search`: what was understood, then one line per photo (date, camera, id).
/// `shoebox-keeper index` (S1-16).
pub fn index(v: &Value) -> String {
    if v.get("written").is_some() {
        return format!(
            "Compacted {} shards into {} ({} rows dropped)\n",
            v["merged"], v["written"], v["dropped"]
        );
    }
    let total = v["total"].as_u64().unwrap_or(0);
    let pct = |n: &Value| {
        (n.as_u64().unwrap_or(0) * 100)
            .checked_div(total)
            .unwrap_or(100)
    };
    let mut out = format!(
        "Search covers {total} photos: by content {}%, labels {}%, text read {}%\n",
        pct(&v["content"]),
        pct(&v["labelled"]),
        pct(&v["text_read"])
    );
    out.push_str(&format!(
        "This keeper: {}{}\n",
        if v["paused"].as_bool() == Some(true) {
            "paused".to_string()
        } else if v["running"].as_bool() == Some(true) {
            format!("indexing ({} photos/min)", v["photos_per_min"])
        } else {
            "not indexing (no image model installed)".to_string()
        },
        if v["meaning"].as_bool() == Some(true) {
            ""
        } else {
            "; no text model, so keyword search only"
        }
    ));
    for i in v["indexers"].as_array().into_iter().flatten() {
        let name = i["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or("a device");
        out.push_str(&format!(
            "  {name}{}: {} photos/min{}\n",
            if i["this"].as_bool() == Some(true) {
                " (this keeper)"
            } else {
                ""
            },
            i["photos_per_min"],
            if i["reads_text"].as_bool() == Some(true) {
                ", reads text"
            } else {
                ""
            }
        ));
    }
    if let Some(eta) = v["eta_secs"].as_u64() {
        out.push_str(&format!("About {} left\n", uptime(eta)));
    }
    let mb = v["held_bytes"].as_u64().unwrap_or(0) as f64 / 1e6;
    out.push_str(&format!("Search data here: {mb:.1} MB\n"));
    out
}

pub fn search(v: &Value) -> String {
    let mut out = String::new();
    let chips: Vec<&str> = v["filters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["label"].as_str())
        .collect();
    let words = v["free_text"].as_str().unwrap_or_default();
    if !chips.is_empty() {
        out.push_str(&format!("Filters: {}\n", chips.join(", ")));
    }
    if !words.is_empty() {
        out.push_str(&format!("Words: {words}\n"));
    }
    let total = v["total"].as_u64().unwrap_or(0);
    let items = v["items"].as_array().cloned().unwrap_or_default();
    out.push_str(&format!(
        "{total} photo{}{}\n",
        if total == 1 { "" } else { "s" },
        if (items.len() as u64) < total {
            format!(", first {}", items.len())
        } else {
            String::new()
        }
    ));
    for it in &items {
        let ms = it["taken_at"].as_i64().unwrap_or(0);
        let d = shoebox_search::Zone::UTC.date(shoebox_core::ids::Timestamp(ms));
        let camera = it["camera"].as_str().unwrap_or("-");
        let id = it["id"].as_str().unwrap_or_default();
        out.push_str(&format!(
            "{}-{:02}-{:02}  {:<24} {}\n",
            d.year,
            d.month,
            d.day,
            camera,
            &id[..id.len().min(16)]
        ));
    }
    out
}

fn gb(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b => format!("{} MB", b.div_ceil(1 << 20)),
    }
}

fn n(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}

/// "Google Photos", "iCloud" or none, for an import's `source`.
fn export_name(v: &Value) -> Option<&'static str> {
    match v["source"].as_str() {
        Some("google") => Some("Google Photos"),
        Some("icloud") => Some("iCloud"),
        _ => None,
    }
}

fn list_missing(out: &mut String, v: &Value, after: &str) {
    let missing: Vec<&str> = v["missing"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !missing.is_empty() {
        let more = if missing.len() > 5 { ", …" } else { "" };
        let _ = writeln!(
            out,
            "  listed in the export but not in these files: {} ({}{more}){after}",
            missing.len(),
            missing[..missing.len().min(5)].join(", ")
        );
    }
}

/// What importing an export would do (`import` before it starts).
pub fn import_plan(v: &Value) -> String {
    let mut out = String::new();
    match export_name(v) {
        Some(name) => {
            let _ = writeln!(out, "An export from {name}.");
        }
        None => out.push_str(
            "These files do not look like a Google Photos or iCloud export: their photos and videos import as plain files.\n",
        ),
    }
    let _ = writeln!(
        out,
        "  {:<24}{}{}",
        "photos and videos",
        n(v, "photos"),
        match n(v, "already") {
            0 => String::new(),
            a => format!(" ({a} more done by an earlier run)"),
        }
    );
    if n(v, "copies") > 0 {
        let _ = writeln!(out, "  {:<24}{} (imported once)", "copies", n(v, "copies"));
    }
    let albums = v["albums"].as_array().cloned().unwrap_or_default();
    if !albums.is_empty() {
        let names: Vec<String> = albums
            .iter()
            .map(|a| format!("{} ({})", s(a, "title"), n(a, "photos")))
            .collect();
        let _ = writeln!(out, "  {:<24}{}", "albums", names.join(", "));
    }
    for (k, label, note) in [
        (
            "with_export_data",
            "with export data",
            " (dates, places, favourites)",
        ),
        ("trashed", "deleted, left out", ""),
        (
            "unmatched",
            "without export data",
            " (they keep what the file says)",
        ),
        ("not_photos", "not photos, skipped", ""),
    ] {
        if n(v, k) > 0 {
            let _ = writeln!(out, "  {label:<24}{}{note}", n(v, k));
        }
    }
    list_missing(
        &mut out,
        v,
        "; if the export came in several parts, give them all",
    );
    for e in v["part_errors"].as_array().into_iter().flatten() {
        let _ = writeln!(out, "  ! could not read {}", e.as_str().unwrap_or("?"));
    }
    let _ = write!(out, "Needs about {} free", gb(n(v, "needed_bytes")));
    match v["free_bytes"].as_u64() {
        Some(free) => {
            let _ = writeln!(out, "; {} free here.", gb(free));
            if free < n(v, "needed_bytes") {
                out.push_str("! Not enough space: free some, or import fewer parts at a time.\n");
            }
        }
        None => out.push_str(".\n"),
    }
    out
}

/// One line of progress for a running import.
pub fn import_progress(v: &Value) -> String {
    match (v["done"].as_u64(), v["total"].as_u64()) {
        (Some(done), Some(total)) if total > 0 => {
            format!("Importing: {done} of {total} ({}%)", done * 100 / total)
        }
        _ => "Importing: reading the files…".into(),
    }
}

/// Where the import stands (`import` with no files), with the last run's report.
pub fn import_status(v: &Value) -> String {
    let mut out = String::new();
    let paths: Vec<&str> = v["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    match s(v, "state") {
        "running" => {
            let _ = writeln!(out, "{}", import_progress(v));
            out.push_str("Stop it with `shoebox-keeper import --stop`; running the same import again continues it.\n");
        }
        "stopped" => {
            let _ = writeln!(
                out,
                "An import stopped before it finished ({} file(s)). Continue it with:\n  shoebox-keeper import {}",
                paths.len(),
                paths.iter().map(|p| shell_quote(p)).collect::<Vec<_>>().join(" ")
            );
            out.push_str("or drop it with `shoebox-keeper import --forget`.\n");
        }
        "idle" => {
            out.push_str("No import. Start one with `shoebox-keeper import <parts or folder>`.\n")
        }
        _ => {}
    }
    if let Some(e) = v["error"].as_str() {
        let _ = writeln!(out, "! {e}");
    }
    if let Some(r) = v.get("report").filter(|r| r.is_object()) {
        out.push_str(&import_report(r));
    }
    out
}

fn shell_quote(p: &str) -> String {
    if p.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        p.to_string()
    } else {
        format!("'{}'", p.replace('\'', r"'\''"))
    }
}

/// What the last finished import did.
pub fn import_report(r: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Imported {} new photos and videos{}.",
        n(r, "imported"),
        match n(r, "duplicates") {
            0 => String::new(),
            d => format!("; {d} already in the library"),
        }
    );
    let albums = r["albums"].as_array().cloned().unwrap_or_default();
    if !albums.is_empty() {
        let names: Vec<String> = albums
            .iter()
            .map(|a| format!("{} ({})", s(a, "title"), n(a, "photos")))
            .collect();
        let _ = writeln!(out, "  {:<24}{}", "albums", names.join(", "));
    }
    for (k, label) in [
        ("resumed", "done by an earlier run"),
        ("export_dates", "dates from the export"),
        ("export_places", "places from the export"),
        ("favorites", "favourites"),
        ("trashed", "left out (deleted)"),
        ("skipped", "not readable as photos"),
        ("unmatched", "without export data"),
    ] {
        if n(r, k) > 0 {
            let _ = writeln!(out, "  {label:<24}{}", n(r, k));
        }
    }
    list_missing(&mut out, r, "");
    let problems: Vec<&str> = r["part_errors"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(r["failed"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    if !problems.is_empty() {
        let _ = writeln!(
            out,
            "! {} problem(s), e.g. {}. Run the same import again to retry them.",
            problems.len(),
            problems[..problems.len().min(3)].join("; ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_import_plan_and_its_report_read_plainly() {
        let plan = json!({
            "source": "icloud", "photos": 12, "already": 0, "copies": 1, "trashed": 1,
            "not_photos": 2, "albums": [{"title": "Trip 2015", "photos": 3}],
            "with_export_data": 12, "unmatched": 0, "missing": ["IMG_0011.JPG"],
            "part_errors": [], "needed_bytes": 3_000_000u64, "free_bytes": 1_000_000u64,
        });
        let text = import_plan(&plan);
        print!("{text}");
        assert!(text.starts_with("An export from iCloud."));
        assert!(text.contains("Trip 2015 (3)"));
        assert!(text.contains("IMG_0011.JPG"));
        assert!(text.contains("Not enough space"));

        let status = json!({
            "state": "stopped", "paths": ["/srv/exports/takeout 1.zip"], "done": null,
            "total": null, "error": null,
            "report": {"imported": 10, "duplicates": 2, "albums": [], "trashed": 1,
                       "missing": [], "part_errors": [], "failed": ["x.jpg: unreadable"]},
        });
        let text = import_status(&status);
        print!("{text}");
        assert!(text.contains("shoebox-keeper import '/srv/exports/takeout 1.zip'"));
        assert!(text.contains("Imported 10 new"));
        assert!(text.contains("1 problem(s)"));
        assert_eq!(
            import_progress(&json!({"done": 5, "total": 20})),
            "Importing: 5 of 20 (25%)"
        );
    }
}
