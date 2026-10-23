//! Explicit, bounded authorization for remote device control.
//!
//! Every operation that goes beyond the read-only commands (`FF`, `FA`,
//! `FC`, `F8`) — today that is only `EC` key injection — must run inside a
//! [`DeviceSession`]: a small JSON file the user starts on purpose with
//! [`DeviceSession::start`], which names
//!
//! * the single allowed operation (`"keys"`),
//! * the exact key names that may be pressed (default: navigation only,
//!   [`NAVIGATION_KEYS`]),
//! * a hard budget of presses ([`DeviceSession::max_presses`]),
//! * an expiry ([`DeviceSession::expires_unix`]).
//!
//! [`DeviceSession::authorize_press`] is the only way to obtain permission
//! for a press: it refuses stopped, expired, over-budget sessions and any
//! key outside the allowlist, and records every granted press in the
//! session file so a session survives CLI invocations.
//!
//! Writes of programs/notes are **not** session operations; they keep using
//! the stronger [`crate::WriteGuard`] preflight (typed confirmation +
//! double absence proof), which a session can never replace.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::keys::{NAVIGATION_KEYS, key_code};

/// The only operation a session may allow today.
pub const OP_KEYS: &str = "keys";

/// One recorded key press.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PressRecord {
    /// Canonical key name.
    pub key: String,
    pub code: u8,
    /// Seconds since the Unix epoch.
    pub at_unix: u64,
}

/// An authorized device-control session, persisted as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSession {
    pub id: String,
    pub operation: String,
    /// Canonical key names that may be pressed, in lower case.
    pub allowed_keys: Vec<String>,
    pub max_presses: usize,
    /// Presses granted so far (append-only).
    pub presses: Vec<PressRecord>,
    /// Seconds since the Unix epoch; refuses after this.
    pub expires_unix: u64,
    pub created_unix: u64,
    pub stopped: bool,
}

impl DeviceSession {
    /// Creates a fresh session. `allowed_keys` defaults to the navigation
    /// keys; unknown names are an error so a typo can never widen access.
    pub fn start(
        id: &str,
        allowed_keys: Option<Vec<String>>,
        max_presses: usize,
        ttl_secs: u64,
    ) -> Result<DeviceSession, String> {
        let allowed: Vec<String> = match allowed_keys {
            None => NAVIGATION_KEYS.iter().map(|s| s.to_string()).collect(),
            Some(names) => {
                let mut out = Vec::new();
                for n in &names {
                    let code = key_code(n).ok_or(format!("unknown key name `{n}`"))?;
                    out.push(
                        crate::keys::key_name(code)
                            .expect("code of a known key")
                            .into(),
                    );
                }
                if out.is_empty() {
                    return Err("an empty allowlist allows nothing; refuse to start".into());
                }
                out
            }
        };
        let now = now_unix();
        Ok(DeviceSession {
            id: id.to_string(),
            operation: OP_KEYS.into(),
            allowed_keys: allowed,
            max_presses,
            presses: Vec::new(),
            expires_unix: now + ttl_secs,
            created_unix: now,
            stopped: false,
        })
    }

    pub fn expired(&self) -> bool {
        now_unix() >= self.expires_unix
    }

    pub fn presses_left(&self) -> usize {
        self.max_presses.saturating_sub(self.presses.len())
    }

    /// Checks that `name` may be pressed in this session right now and
    /// records the grant. This is the *only* press authorization path.
    pub fn authorize_press(&mut self, name: &str) -> Result<u8, String> {
        if self.stopped {
            return Err("session is stopped".into());
        }
        if self.operation != OP_KEYS {
            return Err(format!("session does not allow `{}`", self.operation));
        }
        if self.expired() {
            return Err("session has expired; start a new one".into());
        }
        let Some(def) = crate::keys::key_by_name(name) else {
            return Err(format!("unknown key name `{name}`"));
        };
        if !self.allowed_keys.iter().any(|a| a == def.name) {
            return Err(format!(
                "key `{}` is not in this session's allowlist ({})",
                def.name,
                self.allowed_keys.join(", ")
            ));
        }
        if self.presses_left() == 0 {
            return Err(format!(
                "press budget exhausted ({} of {}); start a new session",
                self.presses.len(),
                self.max_presses
            ));
        }
        self.presses.push(PressRecord {
            key: def.name.into(),
            code: def.code,
            at_unix: now_unix(),
        });
        Ok(def.code)
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    // -- persistence -----------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize session: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn load(path: &Path) -> Result<DeviceSession, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("bad session file: {e}"))
    }

    /// Human-readable one-line status.
    pub fn status_line(&self) -> String {
        let state = if self.stopped {
            "stopped"
        } else if self.expired() {
            "expired"
        } else {
            "active"
        };
        format!(
            "session {} [{}] op={} keys={} used={}/{} expires in {}s",
            self.id,
            state,
            self.operation,
            self.allowed_keys.join(","),
            self.presses.len(),
            self.max_presses,
            self.expires_unix.saturating_sub(now_unix())
        )
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav_session(max: usize) -> DeviceSession {
        DeviceSession::start("s1", None, max, 3600).unwrap()
    }

    #[test]
    fn default_allowlist_is_navigation_only() {
        let mut s = nav_session(10);
        assert!(s.authorize_press("home").is_ok());
        assert!(s.authorize_press("esc").is_ok());
        let e = s.authorize_press("1").unwrap_err();
        assert!(e.contains("allowlist"), "{e}");
        let e = s.authorize_press("shift").unwrap_err();
        assert!(e.contains("allowlist"), "{e}");
        let e = s.authorize_press("on").unwrap_err();
        assert!(e.contains("allowlist"), "{e}");
        let e = s.authorize_press("del").unwrap_err();
        assert!(e.contains("allowlist"), "{e}");
    }

    #[test]
    fn unknown_keys_are_refused_everywhere() {
        let mut s = nav_session(10);
        assert!(s.authorize_press("nope").is_err());
        assert!(DeviceSession::start("s", Some(vec!["nope".into()]), 1, 1).is_err());
        assert!(DeviceSession::start("s", Some(vec![]), 1, 1).is_err());
    }

    #[test]
    fn budget_is_enforced() {
        let mut s = nav_session(2);
        assert!(s.authorize_press("home").is_ok());
        assert!(s.authorize_press("down").is_ok());
        let e = s.authorize_press("up").unwrap_err();
        assert!(e.contains("exhausted"), "{e}");
        assert_eq!(s.presses_left(), 0);
    }

    #[test]
    fn expiry_and_stop_are_enforced() {
        let mut s = DeviceSession::start("s", None, 10, 0).unwrap();
        assert!(s.expired());
        assert!(s.authorize_press("home").unwrap_err().contains("expired"));
        let mut s = nav_session(10);
        s.stop();
        assert!(s.authorize_press("home").unwrap_err().contains("stopped"));
    }

    #[test]
    fn narrower_allowlist_can_be_requested() {
        let mut s =
            DeviceSession::start("s", Some(vec!["Home".into(), "ENTER".into()]), 5, 60).unwrap();
        assert!(s.authorize_press("home").is_ok());
        assert!(s.authorize_press("enter").is_ok());
        assert!(s.authorize_press("esc").is_err());
    }

    #[test]
    fn session_roundtrips_through_json() {
        let dir = std::env::temp_dir().join(format!("zzt-session-{}", std::process::id()));
        let path = dir.join("s.json");
        let mut s = DeviceSession::start("s", None, 3, 3600).unwrap();
        s.authorize_press("home").unwrap();
        s.save(&path).unwrap();
        let loaded = DeviceSession::load(&path).unwrap();
        assert_eq!(s, loaded);
        assert_eq!(loaded.presses.len(), 1);
        assert_eq!(loaded.presses[0].key, "home");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn presses_are_recorded_in_order() {
        let mut s = nav_session(10);
        s.authorize_press("shift").unwrap_err(); // refused: not recorded
        s.authorize_press("home").unwrap();
        s.authorize_press("enter").unwrap();
        let keys: Vec<&str> = s.presses.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, vec!["home", "enter"]);
    }
}
