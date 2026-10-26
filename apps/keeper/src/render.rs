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
