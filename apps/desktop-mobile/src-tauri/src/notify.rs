//! When to tell the user to open the app (M4-10; mockups 5d/5e/5f). Pure rules, no platform code:
//! the native side turns a [`Notification`] into a system notification.
//!
//! A notification is sent only if all hold: photos are at risk, at least one other device is
//! reachable (so opening the app would actually help), background time ran out before the
//! backup finished, and none was sent in the last 24 hours. If nothing is reachable the user
//! gets a banner in the app instead (mockup 5f), never a notification.

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug)]
pub struct NotifyInput {
    /// Photos with fewer copies than the target.
    pub at_risk: u32,
    /// Another device of the library can be reached right now.
    pub peer_reachable: bool,
    /// The last background run stopped because its time budget ended, not because it was done.
    pub budget_exhausted: bool,
    pub last_notified_ms: Option<u64>,
    pub now_ms: u64,
    /// Estimated seconds to finish, if known.
    pub eta_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

fn eta_text(secs: u64) -> String {
    match secs {
        0..=89 => "about a minute".into(),
        90..=3599 => format!("about {} minutes", secs.div_ceil(60)),
        _ => format!("about {} hours", secs.div_ceil(3600)),
    }
}

pub fn decide(i: NotifyInput) -> Option<Notification> {
    let recently = i
        .last_notified_ms
        .is_some_and(|t| i.now_ms.saturating_sub(t) < DAY_MS);
    if i.at_risk == 0 || !i.peer_reachable || !i.budget_exhausted || recently {
        return None;
    }
    let photos = if i.at_risk == 1 {
        "1 photo is".to_string()
    } else {
        format!("{} photos are", i.at_risk)
    };
    let eta = i
        .eta_secs
        .map(|s| format!(" It should take {}.", eta_text(s)))
        .unwrap_or_default();
    Some(Notification {
        title: "Finish backing up".into(),
        body: format!("{photos} not backed up yet. Open Shoebox to finish.{eta}"),
    })
}

/// The system notification for a security alert (R1-06): a new admin, or a replaced recovery
/// phrase. Names the device only; never anything secret.
pub fn security_notification(alert: &crate::types::SecurityAlert) -> Notification {
    if alert.kind == "phrase_replaced" {
        Notification {
            title: "Recovery phrase replaced".into(),
            body: "Your library's recovery phrase was replaced on another device. Open Shoebox if that was not you."
                .into(),
        }
    } else {
        Notification {
            title: "New admin device".into(),
            body: format!(
                "\u{201c}{}\u{201d} became an admin of your library. Open Shoebox if that was not you.",
                alert.device_name
            ),
        }
    }
}

/// The in-app banner of mockup 5f: photos are at risk but nothing can be reached.
pub fn paused_banner(at_risk: u32, peer_reachable: bool) -> Option<&'static str> {
    (at_risk > 0 && !peer_reachable)
        .then_some("Backup paused: none of your devices are reachable. Photos stay safe on this device until one is.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_notifications_name_the_device_and_nothing_secret() {
        let alert = |kind: &str| crate::types::SecurityAlert {
            kind: kind.into(),
            device_id: "ab".repeat(32),
            device_name: "Unknown tablet".into(),
            at_ms: 1.0,
            notified: false,
        };
        let n = security_notification(&alert("admin_added"));
        assert_eq!(n.title, "New admin device");
        assert!(n.body.contains("Unknown tablet"));
        assert!(!n.body.contains(&"ab".repeat(32)), "no device ids");
        let n = security_notification(&alert("phrase_replaced"));
        assert_eq!(n.title, "Recovery phrase replaced");
    }

    fn input() -> NotifyInput {
        NotifyInput {
            at_risk: 212,
            peer_reachable: true,
            budget_exhausted: true,
            last_notified_ms: None,
            now_ms: 10 * DAY_MS,
            eta_secs: Some(600),
        }
    }

    #[test]
    fn notifies_when_everything_lines_up() {
        let n = decide(input()).unwrap();
        assert_eq!(n.title, "Finish backing up");
        assert!(
            n.body.contains("212 photos are not backed up") && n.body.contains("about 10 minutes"),
            "{}",
            n.body
        );
        assert!(decide(NotifyInput {
            at_risk: 1,
            eta_secs: None,
            ..input()
        })
        .unwrap()
        .body
        .starts_with("1 photo is not backed up yet. Open Shoebox to finish."));
    }

    #[test]
    fn stays_quiet_unless_every_condition_holds() {
        assert!(
            decide(NotifyInput {
                at_risk: 0,
                ..input()
            })
            .is_none(),
            "nothing at risk"
        );
        assert!(
            decide(NotifyInput {
                peer_reachable: false,
                ..input()
            })
            .is_none(),
            "nothing reachable: banner only"
        );
        assert!(
            decide(NotifyInput {
                budget_exhausted: false,
                ..input()
            })
            .is_none(),
            "finished or not out of time"
        );
    }

    #[test]
    fn at_most_once_a_day() {
        let i = input();
        assert!(decide(NotifyInput {
            last_notified_ms: Some(i.now_ms - 3_600_000),
            ..i
        })
        .is_none());
        assert!(decide(NotifyInput {
            last_notified_ms: Some(i.now_ms - DAY_MS + 1),
            ..i
        })
        .is_none());
        assert!(decide(NotifyInput {
            last_notified_ms: Some(i.now_ms - DAY_MS),
            ..i
        })
        .is_some());
    }

    #[test]
    fn eta_wording() {
        assert_eq!(eta_text(30), "about a minute");
        assert_eq!(eta_text(95), "about 2 minutes");
        assert_eq!(eta_text(7300), "about 3 hours");
    }

    #[test]
    fn the_paused_banner_is_for_unreachable_devices_only() {
        assert!(paused_banner(5, false).is_some());
        assert!(paused_banner(5, true).is_none());
        assert!(paused_banner(0, false).is_none());
    }
}
