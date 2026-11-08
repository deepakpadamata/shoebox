//! The one answer to "are my photos safe?" (U1-07), computed here and shown with the same words
//! everywhere. The rule itself is the core's: a photo is safe when enough devices hold its
//! original or a keeper does. This module turns that, the devices and what is moving into one of
//! five states.

use crate::types::SafetyStatus;
use std::time::Duration;

/// After this long without a device that could take the photos, "waiting" becomes "at risk".
pub const AT_RISK_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// What the rule needs to know about one device in the library.
#[derive(Clone, Debug)]
pub struct DeviceFact {
    pub name: String,
    /// Keeper or companion: devices that hold originals and could take the photos. A viewer
    /// stores nothing.
    pub stores: bool,
    pub revoked: bool,
    pub is_this_device: bool,
    pub online: bool,
    /// How long ago it was last reachable; `None` when it never has been.
    pub unseen_for: Option<Duration>,
}

#[derive(Clone, Debug)]
pub struct Inputs {
    pub photos: u32,
    /// Photos that are neither on enough devices nor on a keeper.
    pub not_safe: u32,
    pub devices: Vec<DeviceFact>,
    /// Originals are moving between devices right now.
    pub transferring: bool,
}

fn status(state: &str, count: u32, devices: u32) -> SafetyStatus {
    SafetyStatus {
        state: state.into(),
        count,
        devices,
        since_secs: None,
        reason: None,
    }
}

/// `safe`, `single_device`, `busy`, `waiting` or `at_risk`, for display in this order of need:
/// at risk, waiting, busy, single device, safe.
pub fn compute(i: &Inputs) -> SafetyStatus {
    let live: Vec<&DeviceFact> = i.devices.iter().filter(|d| !d.revoked).collect();
    let devices = live.len() as u32;
    // A library on one device is never "backed up", whatever that device is.
    if live.len() <= 1 {
        return status("single_device", i.not_safe, devices);
    }
    if i.not_safe == 0 {
        // Everything is safe; something may still be copying (more copies than the rule needs).
        return if i.transferring {
            status("busy", 0, devices)
        } else {
            status("safe", i.photos, devices)
        };
    }
    // Some photos are not safe yet. Those who could take them:
    let takers: Vec<&&DeviceFact> = live
        .iter()
        .filter(|d| d.stores && !d.is_this_device)
        .collect();
    if takers.iter().any(|d| d.online) {
        // One is reachable: they are on their way (or about to be).
        return status(
            if i.transferring { "busy" } else { "waiting" },
            i.not_safe,
            devices,
        );
    }
    // Nobody who could take them is reachable. The one gone the shortest time decides.
    let nearest = takers
        .iter()
        .min_by_key(|d| d.unseen_for.unwrap_or(Duration::ZERO))
        .copied();
    let gone = nearest.and_then(|d| d.unseen_for).unwrap_or(Duration::ZERO);
    let mut s = status(
        if !takers.is_empty() && gone >= AT_RISK_AFTER {
            "at_risk"
        } else {
            "waiting"
        },
        i.not_safe,
        devices,
    );
    s.since_secs = nearest.map(|_| gone.as_secs() as u32);
    s.reason = nearest.map(|d| d.name.clone());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn dev(
        name: &str,
        stores: bool,
        me: bool,
        online: bool,
        unseen: Option<Duration>,
    ) -> DeviceFact {
        DeviceFact {
            name: name.into(),
            stores,
            revoked: false,
            is_this_device: me,
            online,
            unseen_for: unseen,
        }
    }

    fn inputs(not_safe: u32, devices: Vec<DeviceFact>, transferring: bool) -> Inputs {
        Inputs {
            photos: 100,
            not_safe,
            devices,
            transferring,
        }
    }

    #[test]
    fn two_devices_and_nothing_missing_is_safe() {
        let s = compute(&inputs(
            0,
            vec![
                dev("Phone", true, true, true, None),
                dev("Mac", true, false, true, None),
            ],
            false,
        ));
        assert_eq!(s.state, "safe");
        assert_eq!(s.count, 100);
        assert_eq!(s.devices, 2);
    }

    #[test]
    fn one_keeper_alone_is_a_single_device_never_safe() {
        let s = compute(&inputs(0, vec![dev("Mac", true, true, true, None)], false));
        assert_eq!(s.state, "single_device");
    }

    #[test]
    fn a_phone_alone_is_single_device_even_with_photos_only_on_it() {
        let s = compute(&inputs(
            12,
            vec![dev("Phone", true, true, true, None)],
            false,
        ));
        assert_eq!(s.state, "single_device");
        assert_eq!(s.count, 12);
    }

    #[test]
    fn keeper_offline_for_a_day_is_waiting_and_names_it() {
        let s = compute(&inputs(
            214,
            vec![
                dev("Phone", true, true, true, None),
                dev("Mac Studio", true, false, false, Some(DAY)),
            ],
            false,
        ));
        assert_eq!(s.state, "waiting");
        assert_eq!(s.count, 214);
        assert_eq!(s.reason.as_deref(), Some("Mac Studio"));
        assert_eq!(s.since_secs, Some(86_400));
    }

    #[test]
    fn the_same_after_seven_days_is_at_risk() {
        let s = compute(&inputs(
            214,
            vec![
                dev("Phone", true, true, true, None),
                dev("Mac Studio", true, false, false, Some(7 * DAY)),
            ],
            false,
        ));
        assert_eq!(s.state, "at_risk");
        assert_eq!(s.reason.as_deref(), Some("Mac Studio"));
    }

    #[test]
    fn a_transfer_running_is_busy() {
        let s = compute(&inputs(
            23,
            vec![
                dev("Phone", true, true, true, None),
                dev("Mac", true, false, true, None),
            ],
            true,
        ));
        assert_eq!(s.state, "busy");
        assert_eq!(s.count, 23);
    }

    #[test]
    fn a_revoked_keeper_does_not_count() {
        let mut mac = dev("Mac", true, false, true, None);
        mac.revoked = true;
        let s = compute(&inputs(
            0,
            vec![dev("Phone", true, true, true, None), mac],
            false,
        ));
        assert_eq!(s.state, "single_device");
        assert_eq!(s.devices, 1);
    }

    #[test]
    fn a_viewer_cannot_take_photos() {
        // Phone plus a viewer, a day apart from the viewer: nobody could take the photos, so
        // there is nobody to wait for either; it stays "waiting" until a taker exists.
        let s = compute(&inputs(
            5,
            vec![
                dev("Phone", true, true, true, None),
                dev("Tablet", false, false, false, Some(30 * DAY)),
            ],
            false,
        ));
        assert_eq!(s.state, "waiting");
    }
}
