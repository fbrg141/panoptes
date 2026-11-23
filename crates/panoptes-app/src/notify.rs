//! Native system notifications, identified by the session they open.

use std::process::{Command, Stdio};

use gpui_kit::{App, SystemNotification};

use crate::model::notify::Notice;

/// Builds a notification whose tag takes a click back to this session.
fn request(key: &str, notice: &Notice) -> SystemNotification {
    SystemNotification {
        tag: key.to_string().into(),
        title: notice.title.clone().into(),
        body: notice.body.clone().into(),
        actions: Vec::new(),
    }
}

/// Shows `notice` as Panoptes, with macOS's own icon, permission and click
/// handling. GPUI is a no-op when a bare development binary cannot post.
pub fn post(key: &str, notice: &Notice, cx: &App) {
    cx.show_system_notification(request(key, notice));
}

/// The system alert sound played with a notification. A fixed default rather
/// than a user-chosen file: it is the sound macOS already plays for alerts,
/// so the notification sounds like the rest of the system.
pub const SOUND: &str = "/System/Library/Sounds/Glass.aiff";

/// Plays the alert sound. `afplay` is part of macOS and takes the file as one
/// argument, so no escaping is involved. A missing file or command must not
/// interrupt a session: the notification has already been shown.
pub fn play_sound() {
    let _ = Command::new("afplay")
        .arg(SOUND)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_is_a_system_alert_file() {
        // A fixed path means no user input reaches the command, and the file
        // is the one macOS ships.
        assert!(SOUND.starts_with("/System/Library/Sounds/"));
        assert!(SOUND.ends_with(".aiff"));
        assert!(!SOUND.contains('"') && !SOUND.contains(char::from(92)));
        assert!(std::path::Path::new(SOUND).exists() || !cfg!(target_os = "macos"));
    }

    #[test]
    fn a_notification_carries_its_session_and_text() {
        let request = request(
            "panoptes:42",
            &Notice {
                title: "Run finished".into(),
                body: "Fix login: retry \"auth\"\nnow".into(),
            },
        );
        assert_eq!(request.tag, "panoptes:42");
        assert_eq!(request.title, "Run finished");
        assert_eq!(request.body, "Fix login: retry \"auth\"\nnow");
        assert!(request.actions.is_empty());
    }
}
