//! What is worth a notification: a run finishing, and a session that needs
//! the user. Pure, so the rules can be tested without a window or macOS.

use panoptes_core::store::NotificationPreferences;
use panoptes_harness::SessionStatus;

/// One system notification, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Notice {
    /// The notice for `session` finishing its run.
    fn finished(session: &str) -> Self {
        Self {
            title: "Run finished".into(),
            body: session.into(),
        }
    }

    /// The notice for `session` waiting on the user.
    fn needs_attention(session: &str) -> Self {
        Self {
            title: "Needs you".into(),
            body: session.into(),
        }
    }
}

/// A session is attended only in a window that is actually key. In
/// particular, macOS can retain a main window after another app is frontmost.
pub fn attended(active_key: Option<&str>, key: &str, window_active: bool) -> bool {
    window_active && active_key == Some(key)
}

/// What to notify about a session moving from `previous` to `status`.
///
/// `attended` means the user is looking at that session in the frontmost
/// window, so telling them would only repeat what they can see. A session
/// that keeps the status it had asks for nothing: reopening a session that
/// is still Done is not news.
pub fn notice(
    previous: Option<SessionStatus>,
    status: SessionStatus,
    settings: &NotificationPreferences,
    attended: bool,
    session: &str,
) -> Option<Notice> {
    if !settings.enabled || attended || previous == Some(status) {
        return None;
    }
    match status {
        SessionStatus::Done if settings.finished => Some(Notice::finished(session)),
        SessionStatus::Waiting if settings.needs_attention => {
            Some(Notice::needs_attention(session))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(finished: bool, needs_attention: bool) -> NotificationPreferences {
        NotificationPreferences {
            enabled: true,
            finished,
            needs_attention,
        }
    }

    #[test]
    fn a_background_window_does_not_count_as_attended() {
        assert!(attended(Some("k"), "k", true));
        assert!(!attended(Some("k"), "k", false));
        assert!(!attended(Some("other"), "k", true));
        assert!(!attended(None, "k", true));
    }

    #[test]
    fn a_run_finishing_and_a_waiting_session_are_worth_a_notice() {
        let all = settings(true, true);
        assert_eq!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Done,
                &all,
                false,
                "Fix login"
            ),
            Some(Notice {
                title: "Run finished".into(),
                body: "Fix login".into()
            })
        );
        assert_eq!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Waiting,
                &all,
                false,
                "Fix login"
            ),
            Some(Notice {
                title: "Needs you".into(),
                body: "Fix login".into()
            })
        );
    }

    #[test]
    fn nothing_is_shown_for_a_status_the_user_can_already_see() {
        let all = settings(true, true);
        // The session is on screen in the front window.
        assert_eq!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Done,
                &all,
                true,
                "Fix login"
            ),
            None
        );
        // Running, idle and failed runs say nothing; nor does a repeat.
        for (previous, status) in [
            (Some(SessionStatus::Idle), SessionStatus::Running),
            (Some(SessionStatus::Running), SessionStatus::Running),
            (Some(SessionStatus::Done), SessionStatus::Done),
            (Some(SessionStatus::Running), SessionStatus::Failed),
            (None, SessionStatus::Idle),
        ] {
            assert_eq!(notice(previous, status, &all, false, "Fix login"), None);
        }
    }

    #[test]
    fn each_kind_can_be_turned_off() {
        let finished_only = settings(true, false);
        assert!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Done,
                &finished_only,
                false,
                "s"
            )
            .is_some()
        );
        assert_eq!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Waiting,
                &finished_only,
                false,
                "s"
            ),
            None
        );

        let off = NotificationPreferences {
            enabled: false,
            ..finished_only
        };
        assert_eq!(
            notice(
                Some(SessionStatus::Running),
                SessionStatus::Done,
                &off,
                false,
                "s"
            ),
            None
        );
    }
}
