//! The composer in a real, headless window, driven by keystrokes through
//! GPUI's own dispatch: the key handling these tests cover lives in the
//! order GPUI runs capture handlers, which no pure function can show.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use panoptes_harness::{ModelInfo, SessionSettings};

use super::Composer;
use crate::backend::Backend;
use crate::model::conversation::Conversation;
use crate::store::AppStore;
use crate::ui::Ui;
use crate::views::picker::PickerEvent;

const KEY: &str = "pi:test";

fn model(id: &str, name: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        name: name.into(),
        provider: "test".into(),
        context_window: None,
        images: true,
    }
}

struct Fixture {
    composer: Entity<Composer>,
    store: Entity<AppStore>,
    cx: VisualTestContext,
    /// Models the model list chose, in order.
    chosen: Rc<RefCell<Vec<String>>>,
    _dir: tempfile::TempDir,
}

/// A composer for a session on model One of One and Two, with `draft`
/// typed in its focused field. `stalled` keeps every backend call waiting,
/// so the model list stays open on what the store already has.
fn composer(stalled: bool, busy: bool, draft: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::for_tests(dir.path(), stalled);
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(cx, Some(false));
        super::init(cx);
    });
    let store = app.new(|cx| AppStore::new(backend, cx));
    store.update(&mut app, |store, _| {
        let state = &mut store.state;
        state.settings.insert(
            KEY.into(),
            SessionSettings {
                model: Some(model("test/one", "One")),
                ..Default::default()
            },
        );
        state.models.insert(
            KEY.into(),
            vec![model("test/one", "One"), model("test/two", "Two")],
        );
        state.conversations.insert(
            KEY.into(),
            Conversation {
                busy,
                ..Default::default()
            },
        );
    });
    let ui = app.new(|_| Ui::default());
    let (composer, cx) = app.add_window_view(|window, cx| {
        Composer::new(KEY.into(), store.clone(), ui.clone(), window, cx)
    });
    let mut cx = cx.clone();
    let chosen = Rc::new(RefCell::new(Vec::new()));
    {
        let chosen = chosen.clone();
        let picker = composer.read_with(&cx, |c, _| c.model_picker.clone());
        cx.update(|_, cx| {
            cx.subscribe(&picker, move |_, event, _| {
                if let PickerEvent::Selected(id) = event {
                    chosen.borrow_mut().push(id.clone());
                }
            })
            .detach();
        });
    }
    composer.update_in(&mut cx, |c, window, cx| {
        let focus = c.input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    cx.simulate_input(draft);
    cx.run_until_parked();
    Fixture {
        composer,
        store,
        cx,
        chosen,
        _dir: dir,
    }
}

impl Fixture {
    fn model_list_open(&mut self) -> bool {
        self.composer
            .read_with(&self.cx, |c, cx| c.model_picker.read(cx).is_open())
    }

    fn field_focused(&mut self) -> bool {
        self.composer.update_in(&mut self.cx, |c, window, cx| {
            c.input.read(cx).focus_handle(cx).is_focused(window)
        })
    }

    fn draft(&mut self) -> String {
        self.composer.read_with(&self.cx, |c, cx| c.text(cx))
    }
}

#[::core::prelude::v1::test]
fn enter_in_the_model_list_chooses_the_model_and_keeps_the_draft() {
    let mut f = composer(true, false, "hello agent");
    f.cx.simulate_keystrokes("cmd-m");
    assert!(f.model_list_open(), "cmd-m opens the model list");
    assert!(!f.field_focused(), "the list's search has the focus");

    f.cx.simulate_keystrokes("down enter");
    assert_eq!(*f.chosen.borrow(), ["test/two"], "Enter chose Two");
    assert!(!f.model_list_open(), "choosing closes the list");
    assert_eq!(f.draft(), "hello agent", "the draft was not sent");
    assert!(f.field_focused(), "the caret is back in the field");
}

#[::core::prelude::v1::test]
fn escape_in_the_model_list_closes_it_and_leaves_the_run_alone() {
    let mut f = composer(true, true, "steer this");
    f.cx.simulate_keystrokes("cmd-m");
    assert!(f.model_list_open());

    // Were the composer's Escape to stop the run, it would stop the key
    // first, and the list would stay open.
    f.cx.simulate_keystrokes("escape");
    assert!(!f.model_list_open(), "Escape reached the list");
    assert!(f.field_focused(), "the caret is back in the field");
    assert_eq!(f.draft(), "steer this");
    assert!(f.chosen.borrow().is_empty(), "nothing was chosen");
}

#[::core::prelude::v1::test]
fn keys_in_the_field_still_work_after_the_list_closes() {
    let mut f = composer(true, false, "");
    f.cx.simulate_keystrokes("cmd-m escape");
    assert!(f.field_focused());
    f.cx.simulate_input("typed after");
    assert_eq!(f.draft(), "typed after", "the field takes typing again");
}

#[::core::prelude::v1::test]
fn a_failed_model_load_closes_the_list_and_returns_the_caret() {
    // No harness has this session open, so listing its models fails.
    let mut f = composer(false, false, "draft");
    f.cx.executor().allow_parking();
    f.cx.simulate_keystrokes("cmd-m");
    let deadline = Instant::now() + Duration::from_secs(10);
    while f.model_list_open() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        f.cx.run_until_parked();
    }
    assert!(!f.model_list_open(), "the failed load closed the list");
    assert!(f.field_focused(), "the caret is back in the field");
    let error = f.store.read_with(&f.cx, |s, _| s.state.error.clone());
    assert!(error.is_some(), "the failure is in the error banner");
    f.cx.simulate_input(" more");
    assert_eq!(f.draft(), "draft more");
}
