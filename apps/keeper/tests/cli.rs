//! M2-06: the CLI subcommands against a running daemon over the control socket.

use keeper::{Daemon, KeeperOptions};
use std::process::Command;

fn run(storage: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_shoebox-keeper"))
        .arg("--storage")
        .arg(storage)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subcommands_talk_to_the_running_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("k");
    let d = Daemon::start(KeeperOptions::local_only(&storage, "pi"))
        .await
        .unwrap();
    let storage2 = storage.clone();
    let result = tokio::task::spawn_blocking(move || {
        let (ok, out, _) = run(&storage2, &["status", "--json"]);
        assert!(ok);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["paired"], false);
        assert_eq!(v["name"], "pi");

        let (ok, out, _) = run(&storage2, &["status"]);
        assert!(
            ok && out.contains("Not paired") && out.contains("shoebox-keeper pair"),
            "{out}"
        );

        let (ok, _, err) = run(&storage2, &["devices"]);
        assert!(!ok && err.contains("not paired"), "{err}");

        let (ok, _, err) = run(&storage2, &["album", "list"]);
        assert!(
            !ok && err.contains("available after shared albums"),
            "{err}"
        );

        let (ok, _, err) = run(&storage2, &["pair", "not-a-ticket", "--yes"]);
        assert!(!ok && err.contains("pairing ticket"), "{err}");
    })
    .await;
    result.unwrap();

    // `stop` makes the daemon ask to shut down.
    let storage3 = storage.clone();
    tokio::task::spawn_blocking(move || assert!(run(&storage3, &["stop"]).0))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), d.wait_for_shutdown())
        .await
        .unwrap();
    d.shutdown().await.unwrap();

    // With nothing running the CLI says so plainly.
    let (ok, _, err) = run(&storage, &["status"]);
    assert!(!ok && err.contains("is it running"), "{err}");
}
