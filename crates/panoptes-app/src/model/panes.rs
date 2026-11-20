//! The window's tiling layout: a tree of splits whose leaves each show one
//! session or none. Port of `src/panes.ts`.

use std::sync::atomic::{AtomicU64, Ordering};

/// A vertical split puts panes side by side; a horizontal split stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLeaf {
    pub id: String,
    pub session_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneNode {
    Leaf(PaneLeaf),
    Split {
        id: String,
        direction: SplitDirection,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn next_id(kind: &str) -> String {
    format!("{kind}-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1)
}

pub fn single_pane(session_key: Option<String>) -> PaneNode {
    PaneNode::Leaf(PaneLeaf {
        id: next_id("pane"),
        session_key,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub layout: PaneNode,
    pub focused: String,
}

impl PaneNode {
    pub fn leaves(&self) -> Vec<&PaneLeaf> {
        match self {
            PaneNode::Leaf(leaf) => vec![leaf],
            PaneNode::Split { first, second, .. } => {
                let mut list = first.leaves();
                list.extend(second.leaves());
                list
            }
        }
    }

    pub fn find(&self, pane_id: &str) -> Option<&PaneLeaf> {
        self.leaves().into_iter().find(|p| p.id == pane_id)
    }

    pub fn session_pane(&self, session_key: &str) -> Option<&PaneLeaf> {
        self.leaves()
            .into_iter()
            .find(|p| p.session_key.as_deref() == Some(session_key))
    }

    fn replace(
        &self,
        pane_id: &str,
        replacement: &mut dyn FnMut(&PaneLeaf) -> PaneNode,
    ) -> PaneNode {
        match self {
            PaneNode::Leaf(leaf) if leaf.id == pane_id => replacement(leaf),
            PaneNode::Leaf(_) => self.clone(),
            PaneNode::Split {
                id,
                direction,
                first,
                second,
            } => PaneNode::Split {
                id: id.clone(),
                direction: *direction,
                first: Box::new(first.replace(pane_id, replacement)),
                second: Box::new(second.replace(pane_id, replacement)),
            },
        }
    }

    fn with_session(&self, pane_id: &str, session_key: Option<String>) -> PaneNode {
        self.replace(pane_id, &mut |pane| {
            PaneNode::Leaf(PaneLeaf {
                id: pane.id.clone(),
                session_key: session_key.clone(),
            })
        })
    }

    /// Splits a leaf, keeping its session first, and returns the new empty
    /// leaf's id.
    pub fn split(&self, pane_id: &str, direction: SplitDirection) -> (PaneNode, Option<String>) {
        let new_pane = PaneLeaf {
            id: next_id("pane"),
            session_key: None,
        };
        let mut split = false;
        let layout = self.replace(pane_id, &mut |pane| {
            split = true;
            PaneNode::Split {
                id: next_id("split"),
                direction,
                first: Box::new(PaneNode::Leaf(pane.clone())),
                second: Box::new(PaneNode::Leaf(new_pane.clone())),
            }
        });
        (layout, split.then_some(new_pane.id))
    }

    /// Removes a leaf and collapses its parent. The last leaf becomes empty.
    pub fn close(&self, pane_id: &str) -> PaneNode {
        match self {
            PaneNode::Leaf(leaf) if leaf.id == pane_id => PaneNode::Leaf(PaneLeaf {
                id: leaf.id.clone(),
                session_key: None,
            }),
            PaneNode::Leaf(_) => self.clone(),
            PaneNode::Split {
                id,
                direction,
                first,
                second,
            } => {
                if matches!(first.as_ref(), PaneNode::Leaf(l) if l.id == pane_id) {
                    return second.as_ref().clone();
                }
                if matches!(second.as_ref(), PaneNode::Leaf(l) if l.id == pane_id) {
                    return first.as_ref().clone();
                }
                PaneNode::Split {
                    id: id.clone(),
                    direction: *direction,
                    first: Box::new(first.close(pane_id)),
                    second: Box::new(second.close(pane_id)),
                }
            }
        }
    }

    /// Shows a session once. Picking one already visible focuses its pane;
    /// an empty destination is closed rather than left behind.
    pub fn place(&self, pane_id: &str, session_key: &str) -> Placed {
        let existing = self.session_pane(session_key).map(|p| p.id.clone());
        let Some(destination) = self.find(pane_id) else {
            let focused = existing
                .or_else(|| self.leaves().first().map(|p| p.id.clone()))
                .unwrap_or_else(|| pane_id.to_string());
            return Placed {
                layout: self.clone(),
                focused,
            };
        };
        if let Some(existing) = existing {
            let layout = if existing != pane_id && destination.session_key.is_none() {
                self.close(pane_id)
            } else {
                self.clone()
            };
            return Placed {
                layout,
                focused: existing,
            };
        }
        Placed {
            layout: self.with_session(pane_id, Some(session_key.to_string())),
            focused: pane_id.to_string(),
        }
    }

    /// Drops a session from the sidebar or another pane. Edge drops split
    /// the destination. A centre drop fills an empty pane, replaces an
    /// unrepresented session, or swaps two panes that are already on screen.
    pub fn drop_session(&self, session_key: &str, pane_id: &str, zone: DropZone) -> Placed {
        let unchanged = || Placed {
            layout: self.clone(),
            focused: pane_id.to_string(),
        };
        let Some(destination) = self.find(pane_id) else {
            return unchanged();
        };
        let source = self.session_pane(session_key).map(|p| p.id.clone());
        if source.as_deref() == Some(pane_id) {
            return unchanged();
        }
        let key = Some(session_key.to_string());
        if zone == DropZone::Center || destination.session_key.is_none() {
            let layout = match source {
                None => self.with_session(pane_id, key),
                Some(source) if destination.session_key.is_none() => {
                    self.with_session(pane_id, key).close(&source)
                }
                Some(source) => {
                    let swapped = destination.session_key.clone();
                    self.with_session(pane_id, key)
                        .with_session(&source, swapped)
                }
            };
            return Placed {
                layout,
                focused: pane_id.to_string(),
            };
        }
        let dragged = PaneLeaf {
            id: next_id("pane"),
            session_key: key,
        };
        let dragged_first = matches!(zone, DropZone::Left | DropZone::Top);
        let direction = if matches!(zone, DropZone::Left | DropZone::Right) {
            SplitDirection::Vertical
        } else {
            SplitDirection::Horizontal
        };
        let mut layout = self.replace(pane_id, &mut |pane| {
            let (a, b) = if dragged_first {
                (dragged.clone(), pane.clone())
            } else {
                (pane.clone(), dragged.clone())
            };
            PaneNode::Split {
                id: next_id("split"),
                direction,
                first: Box::new(PaneNode::Leaf(a)),
                second: Box::new(PaneNode::Leaf(b)),
            }
        });
        // Split first so the destination still exists when the old leaf goes.
        if let Some(source) = source {
            layout = layout.close(&source);
        }
        Placed {
            layout,
            focused: dragged.id,
        }
    }
}

/// The drop zone under a pointer, with the outer quarter kept for splits.
/// `x` and `y` are relative to the pane's top left.
pub fn drop_zone_at(width: f32, height: f32, x: f32, y: f32) -> DropZone {
    let px = if width == 0. { 0.5 } else { x / width };
    let py = if height == 0. { 0.5 } else { y / height };
    if px < 0.25 {
        DropZone::Left
    } else if px > 0.75 {
        DropZone::Right
    } else if py < 0.25 {
        DropZone::Top
    } else if py > 0.75 {
        DropZone::Bottom
    } else {
        DropZone::Center
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(node: &PaneNode) -> Vec<Option<&str>> {
        node.leaves()
            .iter()
            .map(|l| l.session_key.as_deref())
            .collect()
    }

    #[test]
    fn splitting_keeps_the_session_first() {
        let root = single_pane(Some("a".into()));
        let id = root.leaves()[0].id.clone();
        let (layout, new_id) = root.split(&id, SplitDirection::Vertical);
        assert_eq!(keys(&layout), [Some("a"), None]);
        assert_eq!(layout.leaves()[1].id, new_id.unwrap());
        assert_eq!(layout.close(&layout.leaves()[1].id.clone()), root);
    }

    #[test]
    fn a_shown_session_focuses_its_pane_and_drops_the_empty_one() {
        let root = single_pane(Some("a".into()));
        let id = root.leaves()[0].id.clone();
        let (layout, empty) = root.split(&id, SplitDirection::Vertical);
        let placed = layout.place(&empty.unwrap(), "a");
        assert_eq!(placed.focused, id);
        assert_eq!(keys(&placed.layout), [Some("a")]);
    }

    #[test]
    fn the_last_pane_empties_instead_of_closing() {
        let root = single_pane(Some("a".into()));
        let id = root.leaves()[0].id.clone();
        assert_eq!(keys(&root.close(&id)), [None]);
    }

    #[test]
    fn drops_swap_split_or_fill() {
        let root = single_pane(Some("a".into()));
        let a = root.leaves()[0].id.clone();
        let placed = root.drop_session("b", &a, DropZone::Left);
        assert_eq!(keys(&placed.layout), [Some("b"), Some("a")]);
        let b = placed.focused.clone();
        let swapped = placed.layout.drop_session("a", &b, DropZone::Center);
        assert_eq!(keys(&swapped.layout), [Some("a"), Some("b")]);
        assert_eq!(drop_zone_at(100., 100., 50., 90.), DropZone::Bottom);
        assert_eq!(drop_zone_at(100., 100., 50., 50.), DropZone::Center);
    }
}
