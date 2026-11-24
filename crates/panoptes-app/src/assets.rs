//! Asset source: the design system's icons and loop mark (`design_assets.rs`),
//! then the component library's own assets.

use std::borrow::Cow;

use gpui_kit::*;

mod generated {
    pub struct LoopGeometry {
        pub body: &'static str,
        pub pupils: &'static str,
        /// Height over width of the mark's view box.
        pub aspect: f64,
    }
    include!("design_assets.rs");
}

pub use generated::{ICONS, LOOP_MARKS, LoopGeometry};

pub struct Assets;

/// The logo's pupil: a disc with its glint cut out (`Pupil` in the design system).
const PUPIL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path fill="black" fill-rule="evenodd" d="M4 0a4 4 0 1 0 0 8a4 4 0 1 0 0-8Z M5.4 1.5a1.05 1.05 0 1 0 0 2.1a1.05 1.05 0 1 0 0-2.1Z"/></svg>"#;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/pupil.svg" {
            return Ok(Some(Cow::Borrowed(PUPIL.as_bytes())));
        }
        if let Some(name) = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
            && let Some((_, svg)) = ICONS.iter().find(|(n, _)| *n == name)
        {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        if let Some(rest) = path.strip_prefix("loop/") {
            let mut parts = rest.split('/');
            let (Some(size), Some(part)) = (parts.next(), parts.next()) else {
                return Ok(None);
            };
            if let Some((_, geometry)) = LOOP_MARKS.iter().find(|(s, _)| *s == size) {
                let svg = match part {
                    "body.svg" => geometry.body,
                    "pupils.svg" => geometry.pupils,
                    _ => return Ok(None),
                };
                return Ok(Some(Cow::Borrowed(svg.as_bytes())));
            }
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut list: Vec<SharedString> = ICONS
            .iter()
            .map(|(name, _)| SharedString::from(format!("icons/{name}.svg")))
            .filter(|p| p.starts_with(path))
            .collect();
        list.extend(gpui_kit::assets::AllAssets.list(path)?);
        Ok(list)
    }
}

/// The loop mark's geometry for a rendered width, as the design system picks it.
pub fn loop_geometry(width: f32) -> (&'static str, &'static LoopGeometry) {
    let size = if width >= 80. {
        "L"
    } else if width >= 32. {
        "M"
    } else {
        "S"
    };
    let (_, geometry) = LOOP_MARKS
        .iter()
        .find(|(s, _)| *s == size)
        .expect("every loop mark size is generated");
    (size, geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn serves_every_icon_the_app_uses() {
        for name in [
            "agent",
            "search",
            "command",
            "settings",
            "folder",
            "git-branch",
            "history",
            "x",
            "check",
            "circle-alert",
            "chevron-down",
            "chevron-right",
            "arrow-up",
            "copy",
            "square",
            "ellipsis",
            "plus",
            "lock",
            "file-diff",
            "external-link",
            "loader-circle",
        ] {
            let svg = Assets.load(&format!("icons/{name}.svg")).unwrap();
            assert!(svg.is_some(), "missing icon {name}");
        }
        assert!(Assets.load("loop/S/body.svg").unwrap().is_some());
        assert!(Assets.load("loop/M/pupils.svg").unwrap().is_some());
    }
}
