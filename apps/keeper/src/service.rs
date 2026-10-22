//! Service files (M2-07): a systemd unit on Linux, a launchd plist on macOS.

use std::path::{Path, PathBuf};

pub struct ServiceSpec {
    /// Absolute path of the `shoebox-keeper` binary.
    pub exe: PathBuf,
    pub storage: PathBuf,
    /// Run as this user (system-wide install only); `None` = the invoking user.
    pub user: Option<String>,
}

pub const LABEL: &str = "org.shoebox.keeper";

/// `systemd` unit text. `system` selects `multi-user.target` (system unit) over `default.target`
/// (user unit).
pub fn systemd_unit(spec: &ServiceSpec, system: bool) -> String {
    let mut u = String::new();
    u.push_str("[Unit]\nDescription=Shoebox keeper (always-on photo backstop)\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\n");
    u.push_str(&format!(
        "ExecStart={} --storage {} run\n",
        quote(&spec.exe),
        quote(&spec.storage)
    ));
    if let (true, Some(user)) = (system, &spec.user) {
        u.push_str(&format!("User={user}\n"));
    }
    u.push_str("Restart=always\nRestartSec=5\nStandardOutput=journal\nStandardError=journal\nSyslogIdentifier=shoebox-keeper\n");
    u.push_str("\n[Install]\n");
    u.push_str(if system {
        "WantedBy=multi-user.target\n"
    } else {
        "WantedBy=default.target\n"
    });
    u
}

fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn launchd_plist(spec: &ServiceSpec, log_dir: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--storage</string>
    <string>{storage}</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{log}/keeper.log</string>
  <key>StandardErrorPath</key><string>{log}/keeper.log</string>
</dict>
</plist>
"#,
        exe = xml(&spec.exe.display().to_string()),
        storage = xml(&spec.storage.display().to_string()),
        log = xml(&log_dir.display().to_string()),
    )
}

/// Where the unit/plist goes: (path, system-wide?).
pub fn unit_path(home: &Path, system: bool) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"))
    } else if system {
        PathBuf::from("/etc/systemd/system/shoebox-keeper.service")
    } else {
        home.join(".config/systemd/user/shoebox-keeper.service")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            exe: "/usr/local/bin/shoebox-keeper".into(),
            storage: "/srv/photo store".into(),
            user: Some("shoebox".into()),
        }
    }

    #[test]
    fn systemd_user_unit_restarts_and_logs_to_the_journal() {
        let u = systemd_unit(&spec(), false);
        assert!(u.contains(
            "ExecStart=/usr/local/bin/shoebox-keeper --storage \"/srv/photo store\" run"
        ));
        assert!(u.contains("Restart=always"));
        assert!(u.contains("StandardOutput=journal"));
        assert!(u.contains("WantedBy=default.target"));
        assert!(
            !u.contains("User="),
            "a user unit runs as the invoking user"
        );
    }

    #[test]
    fn systemd_system_unit_can_run_as_a_dedicated_user() {
        let u = systemd_unit(&spec(), true);
        assert!(u.contains("User=shoebox"));
        assert!(u.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn launchd_plist_is_escaped_and_keeps_the_job_alive() {
        let mut s = spec();
        s.storage = "/Users/a&b/shoebox".into();
        let p = launchd_plist(&s, Path::new("/tmp/logs"));
        assert!(p.contains("<string>/Users/a&amp;b/shoebox</string>"));
        assert!(p.contains("<key>KeepAlive</key><true/>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains("/tmp/logs/keeper.log"));
    }

    #[test]
    fn unit_paths() {
        let home = Path::new("/home/pi");
        if !cfg!(target_os = "macos") {
            assert_eq!(
                unit_path(home, false),
                Path::new("/home/pi/.config/systemd/user/shoebox-keeper.service")
            );
            assert_eq!(
                unit_path(home, true),
                Path::new("/etc/systemd/system/shoebox-keeper.service")
            );
        }
    }
}
