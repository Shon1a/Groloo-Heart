//! Per-account install state and its cross-device reconciliation.
//!
//! Install state is deliberately *not* part of the add-on collection — the
//! collection carries only a default hint. Real state is this map, keyed by
//! add-on id, synced last-write-wins by timestamp. Keeping it here (not in the
//! CDN data) is what lets one collection serve every account.
//!
//! VENDORED VERBATIM from `Stredio-Heart@0.1.0 src/state.rs`, tests included.
//! Note that no GROLOO shell reaches this logic today: `/api/addon-state` exists
//! server-side and has zero clients, and install state lives device-local in
//! `groloo.homeconfig`. These three functions are the machinery a shared
//! living-room TV needs — they are kept because the reconciliation rule is the
//! hard part and it is already written and already tested, not because anything
//! currently calls them.

use crate::types::AddonDescriptor;
use std::collections::BTreeMap;

/// `id -> installed?` — ordered so serialisation is deterministic across devices.
pub type InstallMap = BTreeMap<String, bool>;

/// Build the install map from a descriptor list (skipping locked/default entries).
pub fn install_map(addons: &[AddonDescriptor]) -> InstallMap {
    let mut m = InstallMap::new();
    for a in addons {
        if a.locked == Some(true) {
            continue;
        }
        m.insert(a.id.clone(), a.installed.unwrap_or(false));
    }
    m
}

/// Overlay a persisted map onto descriptors: a stored toggle wins over the
/// descriptor's default for every non-locked id (the local-storage layer).
pub fn apply_install_map(addons: &mut [AddonDescriptor], map: &InstallMap) {
    for a in addons.iter_mut() {
        if a.locked == Some(true) {
            continue;
        }
        if let Some(&v) = map.get(&a.id) {
            a.installed = Some(v);
        }
    }
}

/// The outcome of comparing local vs remote state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDecision {
    /// Remote is newer (or a different account owns local) → adopt remote.
    AdoptRemote,
    /// We own local and it is newer/first → push local up.
    UploadLocal,
    /// Nothing to do.
    Noop,
}

/// Decide how to reconcile local and remote install state.
///
/// Mirrors the client's last-write-wins rule: adopt remote when it is newer or a
/// different account previously owned the local copy; otherwise, when we still
/// own it and local is newer-or-equal (or there is no remote), upload.
pub fn reconcile(
    local_at: u64,
    remote_present: bool,
    remote_at: u64,
    owner_changed: bool,
) -> SyncDecision {
    if remote_present && (remote_at > local_at || owner_changed) {
        SyncDecision::AdoptRemote
    } else if !owner_changed && (!remote_present || local_at >= remote_at) {
        SyncDecision::UploadLocal
    } else {
        SyncDecision::Noop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addons() -> Vec<AddonDescriptor> {
        serde_json::from_str(
            r#"[
              {"id":"a","installed":true},
              {"id":"b","installed":false},
              {"id":"c","installed":true,"locked":true}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn map_skips_locked() {
        let m = install_map(&addons());
        assert_eq!(m.get("a"), Some(&true));
        assert_eq!(m.get("b"), Some(&false));
        assert_eq!(m.get("c"), None); // locked excluded
    }

    #[test]
    fn overlay_beats_default_but_not_locked() {
        let mut a = addons();
        let mut map = InstallMap::new();
        map.insert("a".into(), false);
        map.insert("c".into(), false); // locked → ignored
        apply_install_map(&mut a, &map);
        assert_eq!(
            a.iter().find(|x| x.id == "a").unwrap().installed,
            Some(false)
        );
        assert_eq!(
            a.iter().find(|x| x.id == "c").unwrap().installed,
            Some(true)
        );
    }

    #[test]
    fn reconcile_rules() {
        // remote newer → adopt
        assert_eq!(reconcile(10, true, 20, false), SyncDecision::AdoptRemote);
        // different owner → adopt even if not newer
        assert_eq!(reconcile(30, true, 20, true), SyncDecision::AdoptRemote);
        // we own, local newer → upload
        assert_eq!(reconcile(30, true, 20, false), SyncDecision::UploadLocal);
        // no remote, we own → upload
        assert_eq!(reconcile(0, false, 0, false), SyncDecision::UploadLocal);
        // different owner, no remote → noop (nothing to adopt, not ours to push)
        assert_eq!(reconcile(0, false, 0, true), SyncDecision::Noop);
    }
}
