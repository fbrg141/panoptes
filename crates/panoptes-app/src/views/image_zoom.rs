//! An image opened from the transcript, large in a layer over the window.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{self, Type as _};
use crate::theme::PaletteExt as _;
use crate::ui::Ui;
use crate::views::turn::{ZoomedImage, decoded};
use crate::zoom::px;

pub struct ImageZoom {
    ui: Entity<Ui>,
    image: ZoomedImage,
}

impl ImageZoom {
    pub fn new(
        ui: Entity<Ui>,
        image: ZoomedImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self { ui, image }
    }

    fn close(&mut self, cx: &mut Context<Self>) -> bool {
        self.ui.update(cx, |ui, cx| ui.close_image(cx));
        true
    }
}

impl Render for ImageZoom {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let this = cx.entity().downgrade();
        let close: ds::CloseHandler = {
            let this = this.clone();
            std::rc::Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| this.close(cx));
            })
        };
        let body = match decoded(&self.image.mime_type, &self.image.data) {
            Some(image) => div()
                .id("image-zoom-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    // Shown at its own size, not scaled down: the thumbnail is
                    // 240px wide, and a tall screenshot needs to be readable.
                    img(image).object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            None => div()
                .p(px(16.))
                .t_caption()
                .text_color(p.ink_muted)
                .child(format!("IMAGE ({})", self.image.mime_type.to_uppercase()))
                .into_any_element(),
        };
        let panel = ds::overlay_panel(880., window, &p)
            .child(ds::overlay_header(
                div()
                    .t_label()
                    .text_color(p.ink_muted)
                    .child(format!("Image ({})", self.image.mime_type)),
                Some(close),
            ))
            .child(body);
        ds::modal_layer(
            window,
            move |_, cx| this.update(cx, |this, cx| this.close(cx)).unwrap_or(false),
            panel,
            &p,
        )
    }
}
