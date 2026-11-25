//! Panoptes: the window, its backend and the theme, in GPUI.

mod assets;
mod backend;
mod ds;
mod model;
mod notify;
mod store;
mod theme;
mod ui;
mod views;
mod workspace;
mod zoom;

use gpui_kit::*;

fn main() {
    // SAFETY: nothing has started a thread yet.
    unsafe { panoptes_core::git::forget_inherited_repository() };
    // A terminal-started debug build inherits the PATH; an installed app does not.
    if !cfg!(debug_assertions) {
        panoptes_core::shell_path::apply();
    }
    let started = backend::Backend::start();

    gpui_kit::application()
        .with_assets(assets::Assets)
        // Panoptes is a single-window app without dock-reopen support, so
        // closing the last window quits instead of leaving a headless app
        // that no click brings back.
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("dev.panoptes.app", "Panoptes");
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
            zoom::set_zoom(started.preferences.font_size as f32 / 100.);
            theme::init(cx, started.preferences.dark_mode);
            views::composer::init(cx);
            views::shortcuts::init(cx);
            views::review::init(cx);
            // The menu's key equivalents are looked up from the bindings that
            // the inits above registered, so it is set after them.
            cx.set_menus(views::shortcuts::menus());
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
                // The palette follows the system appearance as it changes.
                window
                    .observe_window_appearance(|window, cx| theme::follow(window.appearance(), cx))
                    .detach();
                cx.new(|cx| workspace::Workspace::new(backend, events, window, cx))
            })
            .expect("failed to open the window");
            cx.activate(true);
        });
}
