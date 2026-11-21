//! The keyboard cheatsheet: every shortcut the app responds to, in one place so
//! the key bindings and the cheatsheet never drift apart.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub label: &'static str,
    pub keys: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutGroup {
    pub label: &'static str,
    pub entries: Vec<ShortcutEntry>,
}

const fn entry(label: &'static str, keys: &'static [&'static str]) -> ShortcutEntry {
    ShortcutEntry { label, keys }
}

/// Every shortcut the app responds to, as the cheatsheet shows it.
pub fn shortcut_groups() -> Vec<ShortcutGroup> {
    vec![
        ShortcutGroup {
            label: "Panoptes",
            entries: vec![
                entry("Search projects, lanes and sessions", &["⌘", "P"]),
                entry("New lane", &["⌘", "T"]),
                entry("New session in current lane", &["⌘", "⇧", "N"]),
                entry("Add a project", &["⌘", "N"]),
                entry("Split pane vertically (side by side)", &["⌘/Ctrl", "D"]),
                entry("Split pane horizontally (stacked)", &["⌘/Ctrl", "⇧", "D"]),
                entry("Keyboard shortcuts", &["⌘", "/"]),
            ],
        },
        ShortcutGroup {
            label: "Composer",
            entries: vec![
                entry("Send the message", &["↵"]),
                entry("New line", &["⇧", "↵"]),
                entry("Queue for after the run", &["⌥", "↵"]),
                entry("Stop the agent", &["Esc"]),
                entry("Mention a file", &["@"]),
                entry("Cycle the thinking level", &["⇧", "Tab"]),
            ],
        },
        ShortcutGroup {
            label: "Reading",
            entries: vec![entry("Jump to the latest output", &["⌘", "↓"])],
        },
        ShortcutGroup {
            label: "Zoom",
            entries: vec![
                entry("Zoom in or out", &["⌘", "+/−"]),
                entry("Reset the zoom", &["⌘", "0"]),
            ],
        },
    ]
}

/// The cheatsheet narrowed to `query`: an entry stays when its label or its
/// group's name contains every word typed, and groups left empty drop out.
pub fn filter_shortcuts(groups: Vec<ShortcutGroup>, query: &str) -> Vec<ShortcutGroup> {
    let query = query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return groups;
    }
    groups
        .into_iter()
        .map(|group| ShortcutGroup {
            entries: group
                .entries
                .into_iter()
                .filter(|entry| {
                    let text = format!("{} {}", group.label, entry.label).to_lowercase();
                    words.iter().all(|word| text.contains(word))
                })
                .collect(),
            ..group
        })
        .filter(|group| !group.entries.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_keeps_everything() {
        assert_eq!(filter_shortcuts(shortcut_groups(), "  "), shortcut_groups());
    }

    #[test]
    fn every_word_matches_the_entry_or_its_group_and_empty_groups_drop() {
        let found = filter_shortcuts(shortcut_groups(), "composer QUEUE");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].entries[0].label, "Queue for after the run");
        let split = filter_shortcuts(shortcut_groups(), "split pane");
        assert_eq!(split[0].entries.len(), 2);
        assert!(filter_shortcuts(shortcut_groups(), "nothing like this").is_empty());
    }
}
