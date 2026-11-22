//! Native system notifications, identified by the session they open.

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

#[cfg(test)]
mod tests {
    use super::*;

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
