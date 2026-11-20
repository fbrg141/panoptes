//! Panoptes in GPUI: the walking skeleton of #222. A window with the lane
//! list read from the store, and one live session pane.

mod assets;
mod backend;
mod ds;
mod model;
mod theme;
mod workspace;

use gpui_kit::*;

fn main() {
    // SAFETY: nothing has started a thread yet.
    unsafe { panoptes_core::git::forget_inherited_repository() };
    // A terminal-started debug build inherits the PATH; an installed app does not.
    if !cfg!(debug_assertions) {
        panoptes_core::shell_path::apply();
    }
    let started = backend::Backend::start();

    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                // Starting without the store would show an empty app and
                // risk overwriting what could not be read, so stop instead.
                eprintln!("Panoptes could not start: {error:#}");
                cx.quit();
                return;
            }
        };
        let backend = started.backend.clone();
        cx.on_app_quit(move |_| {
            let backend = backend.clone();
            async move { backend.close_all() }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1200.), px(800.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Panoptes".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut events = Some(started.events);
        let backend = started.backend;
        gpui_kit::open_window(options, cx, move |window, cx| {
            let events = events.take().expect("the window opens once");
            cx.new(|cx| workspace::Workspace::new(backend, events, window, cx))
        })
        .expect("failed to open the window");
        cx.activate(true);
    });
}
