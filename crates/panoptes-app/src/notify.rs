//! Posting a [`Notice`] to macOS.
//!
//! `osascript` shows it as Panoptes' own script notification, without a
//! bundle identifier or a notification permission prompt, and it works for a
//! `cargo run` binary as well as the packaged app. The command is built and
//! tested as a string so the escaping is not a matter of trust.

use std::process::{Command, Stdio};

use crate::model::notify::Notice;

/// Shows `notice`, if the platform lets us. A notification is a nicety: a
/// missing `osascript` must never interrupt a session.
pub fn post(notice: &Notice) {
    if let Ok(mut child) = Command::new("osascript")
        .arg("-e")
        .arg(script(notice))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        // Dropping a Child does not reap it. Notifications are rare, so a
        // small detached waiter keeps a long-running app free of zombies.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// The AppleScript that displays `notice`.
pub fn script(notice: &Notice) -> String {
    format!(
        "display notification \"{}\" with title \"{}\"",
        quoted(&notice.body),
        quoted(&notice.title)
    )
}

/// `text` as the inside of an AppleScript string literal. Notifications put a
/// line or two of the session into the body, and a session title may well
/// hold quotes or a backslash.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            // The body is one line in a banner; newlines would only be lost.
            '\n' | '\r' => out.push(' '),
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(title: &str, body: &str) -> Notice {
        Notice {
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn the_script_names_the_title_and_escapes_the_body() {
        assert_eq!(
            script(&notice("Run finished", "Fix login")),
            "display notification \"Fix login\" with title \"Run finished\""
        );
        // Quotes and backslashes cannot end the literal early, and a
        // two-line body stays on one line.
        assert_eq!(
            script(&notice("Say \"hi\"", "a \"b\" \\ c\nd")),
            "display notification \"a \\\"b\\\" \\\\ c d\" with title \"Say \\\"hi\\\"\""
        );
    }
}
