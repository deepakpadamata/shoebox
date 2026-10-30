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
