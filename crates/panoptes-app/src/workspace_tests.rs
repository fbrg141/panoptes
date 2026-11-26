//! The workspace in a real, headless window, driven by keystrokes: ⌘W
//! closes the window, which quits the app and stops every session, so a
//! session that is Running or Waiting makes it ask first.

use gpui_kit::*;
use panoptes_harness::SessionStatus;
use tokio::sync::mpsc;

use super::Workspace;
use crate::backend::Backend;
use crate::store::Startup;

struct Fixture {
    workspace: Entity<Workspace>,
    cx: VisualTestContext,
    _dir: tempfile::TempDir,
}

/// A workspace past startup, with one session in `status`, and the focus in
/// it. The backend never answers, so nothing reloads over that state.
fn workspace(status: SessionStatus) -> Fixture {
    workspace_at(Startup::Ready, status)
}

fn workspace_at(startup: Startup, status: SessionStatus) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::for_tests(dir.path(), true);
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(cx, Some(false));
        crate::views::composer::init(cx);
        crate::views::shortcuts::init(cx);
        crate::views::review::init(cx);
    });
    let (_sender, events) = mpsc::unbounded_channel();
    let (workspace, cx) =
        app.add_window_view(|window, cx| Workspace::new(backend, events, window, cx));
    let mut cx = cx.clone();
    workspace.update_in(&mut cx, |this, window, cx| {
        this.store.update(cx, |store, cx| {
            store.state.startup = startup;
            store.state.statuses.insert("pi:one".into(), status);
            cx.notify();
        });
        window.focus(&this.focus, cx);
    });
    cx.run_until_parked();
    Fixture {
        workspace,
        cx,
        _dir: dir,
    }
}

impl Fixture {
    fn window_open(&self) -> bool {
        !self.cx.windows().is_empty()
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }
}

#[::core::prelude::v1::test]
fn cmd_w_closes_at_once_when_nothing_runs() {
    for status in [
        SessionStatus::Idle,
        SessionStatus::Done,
        SessionStatus::Failed,
    ] {
        let mut f = workspace(status);
        f.press("cmd-w");
        assert!(!f.cx.has_pending_prompt(), "{status:?}: no question");
        assert!(!f.window_open(), "{status:?}: the window closed");
    }
}

#[::core::prelude::v1::test]
fn cmd_w_asks_first_while_a_session_runs_or_waits() {
    for status in [SessionStatus::Running, SessionStatus::Waiting] {
        let mut f = workspace(status);
        f.press("cmd-w");
        assert!(f.cx.has_pending_prompt(), "{status:?}: it asks");
        assert!(f.window_open(), "{status:?}: nothing closed yet");
        let (message, _) = f.cx.pending_prompt().unwrap();
        assert_eq!(message, "Close Panoptes?");

        f.cx.simulate_prompt_answer("Cancel");
        f.cx.run_until_parked();
        assert!(f.window_open(), "{status:?}: Cancel keeps the window");
        assert!(
            f.workspace
                .read_with(&f.cx, |w, cx| w.store.read(cx).state.has_active_run()),
            "{status:?}: the session is left as it was"
        );

        f.press("cmd-w");
        f.cx.simulate_prompt_answer("Close");
        f.cx.run_until_parked();
        assert!(!f.window_open(), "{status:?}: Close closes the window");
    }
}

#[::core::prelude::v1::test]
fn cmd_w_closes_the_startup_screen_too() {
    let mut f = workspace_at(Startup::Loading, SessionStatus::Idle);
    f.press("cmd-w");
    assert!(!f.window_open(), "the window closed while loading");
}
