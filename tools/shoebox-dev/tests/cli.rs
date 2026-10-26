//! M1-07: the developer CLI, driven as a real process. One device, then two devices talking over
//! real (local) connections.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_shoebox-dev");

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples")
}

/// `shoebox-dev --dir <dir> --local-only <args>`
fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("--dir")
        .arg(dir)
        .arg("--local-only")
        .args(args)
        .output()
        .expect("runs the CLI")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = cli(dir, args);
    assert!(
        out.status.success(),
        "`{}` failed: {}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    serde_json::from_str(&ok(dir, &full)).expect("--json prints JSON")
}

#[test]
fn a_single_device_can_init_import_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("laptop");

    // Nothing works before `init`, and the error says what to do.
    let out = cli(&dir, &["ls"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("init"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    ok(&dir, &["init", "--name", "laptop"]);
    assert_eq!(json(&dir, &["ls"]), serde_json::json!([]));
    assert!(
        !cli(&dir, &["init"]).status.success(),
        "init twice is refused"
    );

    let s = samples();
    let report = json(
        &dir,
        &[
            "import",
            s.join("photo-gps.jpg").to_str().unwrap(),
            s.join("screenshot.png").to_str().unwrap(),
        ],
    );
    assert_eq!(report["imported"], 2);
    assert_eq!(report["duplicates"], 0);
    // Importing again finds nothing new.
    let again = json(&dir, &["import", s.join("photo-gps.jpg").to_str().unwrap()]);
    assert_eq!(
        (again["imported"].as_u64(), again["duplicates"].as_u64()),
        (Some(0), Some(1))
    );

    let list = json(&dir, &["ls"]);
    let photos = list.as_array().unwrap();
    assert_eq!(photos.len(), 2);
    for p in photos {
        assert_eq!(p["copies"], 1, "only this device holds it");
        assert_eq!(
            p["local"],
            serde_json::json!({"thumb": true, "preview": true, "original": true})
        );
        assert!(p["id"].as_str().unwrap().len() == 64);
    }
    // Newest first: the GPS photo (2024) before the screenshot (file mtime = now) is not
    // guaranteed, but the order must be by taken_at descending.
    let times: Vec<i64> = photos
        .iter()
        .map(|p| p["taken_at"].as_i64().unwrap())
        .collect();
    assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");

    let id = photos[0]["id"].as_str().unwrap();
    let holdings = json(&dir, &["holdings", &id[..10]]);
    assert_eq!(holdings["copies"], 1);
    assert_eq!(holdings["target"], 2);
    assert_eq!(holdings["holders"].as_array().unwrap().len(), 1);
    assert!(
        cli(&dir, &["holdings", "zz"]).status.code() != Some(0),
        "unknown prefix is an error"
    );

    let status = json(&dir, &["status"]);
    assert_eq!(status["photos"], 2);
    // One copy is below the target of 2, but it is on a keeper, which counts as backed up.
    assert_eq!(status["at_risk"], 0, "held by a keeper");
    assert_eq!(status["role"], "Keeper");
}

struct Daemon {
    child: Child,
    ticket: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start `run` and read the ticket it prints.
fn start_daemon(dir: &Path) -> Daemon {
    let child = Command::new(BIN)
        .arg("--dir")
        .arg(dir)
        .arg("--local-only")
        .arg("run")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("starts the daemon");
    // From here on `Drop` kills and reaps the process even if an assertion below panics.
    let mut daemon = Daemon {
        child,
        ticket: String::new(),
    };
    let stdout = daemon.child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "the daemon did not print a ticket"
        );
        let line = lines.next().expect("daemon output").expect("utf8");
        if let Some(ticket) = line.strip_prefix("ticket: ") {
            // Keep draining stdout in the background so the daemon never blocks on a full pipe.
            std::thread::spawn(move || for _ in lines {});
            daemon.ticket = ticket.trim().to_string();
            return daemon;
        }
    }
}

#[test]
fn two_devices_join_sync_and_fetch_over_real_connections() {
    let tmp = tempfile::tempdir().unwrap();
    let (laptop, pi) = (tmp.path().join("laptop"), tmp.path().join("pi"));

    ok(&laptop, &["init", "--name", "laptop"]);
    let s = samples();
    ok(
        &laptop,
        &[
            "import",
            s.join("photo-gps.jpg").to_str().unwrap(),
            s.join("IMG_0001.jpg").to_str().unwrap(),
        ],
    );
    let daemon = start_daemon(&laptop);

    // The Pi (a keeper) joins with the ticket and syncs once.
    ok(
        &pi,
        &["join", &daemon.ticket, "--name", "pi", "--role", "keeper"],
    );
    let list = json(&pi, &["ls"]);
    assert_eq!(list.as_array().unwrap().len(), 2, "the library arrived");
    let id = list[0]["id"].as_str().unwrap().to_string();

    // It has the metadata but not yet the bytes; fetching an original pulls it from the laptop.
    assert_eq!(list[0]["local"]["original"], false);
    let fetched = json(&pi, &["fetch", &id[..12], "--kind", "original"]);
    assert_eq!(fetched["kind"], "original");
    assert!(
        fetched["from"].as_str().unwrap().len() >= 8,
        "names the source device"
    );
    let after = json(&pi, &["ls"]);
    let rec = after
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id.as_str())
        .unwrap();
    assert_eq!(rec["local"]["original"], true);
    assert_eq!(rec["copies"], 2, "laptop + pi");

    // The Pi knows the laptop and can reach it.
    let peers = json(&pi, &["peers"]);
    let peers = peers.as_array().unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["reachable"], true);
    assert!(peers[0]["latency_ms"].is_number());
    drop(daemon);
}
