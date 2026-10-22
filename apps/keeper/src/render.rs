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
