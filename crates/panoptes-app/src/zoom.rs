//! Window zoom driven by ⌘+/- and ⌘0. GPUI has no page zoom, so every length
//! the app sets goes through this module's `px`, which scales by the current
//! zoom; views import it in place of GPUI's.

use std::sync::atomic::{AtomicU32, Ordering};

pub const MIN_ZOOM: f32 = 0.5;
pub const MAX_ZOOM: f32 = 2.0;
const STEP: f32 = 0.1;

static ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomCommand {
    In,
    Out,
    Reset,
}

pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(Ordering::Relaxed))
}

pub fn set_zoom(scale: f32) {
    ZOOM.store(scale.to_bits(), Ordering::Relaxed);
}

/// The next zoom scale, clamped to [MIN_ZOOM, MAX_ZOOM] in tenths.
pub fn next_zoom(current: f32, command: ZoomCommand) -> f32 {
    let target = match command {
        ZoomCommand::Reset => 1.0,
        ZoomCommand::In => current + STEP,
        ZoomCommand::Out => current - STEP,
    };
    ((target * 10.0).round() / 10.0).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// A length in the design's pixels, at the window's zoom.
pub fn px(pixels: f32) -> gpui_kit::Pixels {
    gpui_kit::px(pixels * zoom())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_by_tenths_within_bounds() {
        assert_eq!(next_zoom(1.0, ZoomCommand::In), 1.1);
        assert_eq!(next_zoom(1.0, ZoomCommand::Out), 0.9);
        assert_eq!(next_zoom(1.7, ZoomCommand::Reset), 1.0);
        assert_eq!(next_zoom(2.0, ZoomCommand::In), 2.0);
        assert_eq!(next_zoom(0.5, ZoomCommand::Out), 0.5);
        // Float drift does not accumulate.
        let mut z = 1.0;
        for _ in 0..3 {
            z = next_zoom(z, ZoomCommand::In);
        }
        assert_eq!(z, 1.3);
    }
}
