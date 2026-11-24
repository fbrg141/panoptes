//! Native system notifications, identified by the session they open, and the
//! sound Panoptes plays alongside them.

use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

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

/// The sound Panoptes plays with a notification: macOS's Glass alert file,
/// played through `afplay`.
///
/// It is a sound of our own rather than the notification's own, because
/// GPUI's `SystemNotification` carries no sound and its macOS backend never
/// sets one, so a notification there arrives silently. It is therefore not
/// the user's chosen alert sound (Sound settings), it plays at output volume
/// rather than alert volume, and `afplay` runs outside the notification
/// system, so it does not follow Focus or Do Not Disturb.
pub const SOUND: &str = "/System/Library/Sounds/Glass.aiff";

/// Set while a sound is playing, so quick notifications, and repeated clicks
/// on the preview button, cannot overlap into one indistinguishable chime.
static PLAYING: AtomicBool = AtomicBool::new(false);

/// Plays the alert sound unless one is still playing. A missing file or
/// command must not interrupt a session: the notification has already been
/// shown, so a failed playback is ignored.
pub fn play_sound() {
    let _ = play(&PLAYING, afplay(SOUND));
}

/// `afplay` takes the sound file as one argument, so no escaping is involved.
/// Its output is dropped: nothing reads it, and a full pipe could block the
/// child.
fn afplay(sound: &str) -> Command {
    let mut command = Command::new("afplay");
    command
        .arg(sound)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Runs `command` in the background, or returns `None` when a sound is still
/// playing. The child is reaped on a worker thread: on Unix a dropped `Child`
/// becomes a zombie until the app quits, and the UI thread must not wait.
fn play(
    playing: &'static AtomicBool,
    mut command: Command,
) -> Option<JoinHandle<Option<ExitStatus>>> {
    if playing.swap(true, Ordering::SeqCst) {
        return None;
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            playing.store(false, Ordering::SeqCst);
            return None;
        }
    };
    Some(std::thread::spawn(move || {
        let status = child.wait().ok();
        playing.store(false, Ordering::SeqCst);
        status
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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
                sound: true,
            },
        );
        assert_eq!(request.tag, "panoptes:42");
        assert_eq!(request.title, "Run finished");
        assert_eq!(request.body, "Fix login: retry \"auth\"\nnow");
        assert!(request.actions.is_empty());
    }

    /// Its own flag, so the tests never share the playing state.
    fn idle() -> &'static AtomicBool {
        Box::leak(Box::new(AtomicBool::new(false)))
    }

    /// The sound is held for `seconds`, so a second attempt meets the first.
    fn lasting(seconds: f32) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &format!("sleep {seconds}")]);
        command
    }

    #[test]
    fn a_played_sound_is_reaped_rather_than_left_a_zombie() {
        // `true` exits at once; joining the reaper proves the child was waited
        // on, and the status proves it really ran.
        let reaper = play(idle(), Command::new("/usr/bin/true")).unwrap();
        assert!(reaper.join().unwrap().unwrap().success());
    }

    #[test]
    fn a_missing_command_is_ignored_and_releases_the_flag() {
        let playing = idle();
        assert!(play(playing, Command::new("/nonexistent/afplay")).is_none());
        assert!(play(playing, lasting(0.0)).is_some());
    }

    #[test]
    fn only_one_sound_plays_at_a_time() {
        let playing = idle();
        let first = play(playing, lasting(0.2)).unwrap();
        assert!(play(playing, lasting(0.0)).is_none());
        first.join().unwrap().unwrap();
        // The flag is released once the sound has finished.
        let deadline = Instant::now() + Duration::from_secs(5);
        while playing.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the sound never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(play(playing, lasting(0.0)).is_some());
    }
}
