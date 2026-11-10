import { Icon, Kbd } from "@panoptes/design-system";
import { useMemo, useState } from "react";
import { filterShortcuts, SHORTCUT_GROUPS } from "../shortcuts";
import { useUi } from "../ui";
import { Overlay, OverlayHeader } from "./Overlay";

/**
 * The ⌘/ cheatsheet: every shortcut the app responds to, laid out in two
 * columns so it reads at a glance, with a filter for when it doesn't.
 */
export function ShortcutsDialog() {
  const close = useUi((s) => s.close);
  const [query, setQuery] = useState("");
  const groups = useMemo(() => filterShortcuts(SHORTCUT_GROUPS, query), [query]);

  return (
    <Overlay label="Keyboard shortcuts" width={680} onClose={close}>
      <OverlayHeader onClose={close}>
        <h2 className="text-heading">Keyboard shortcuts</h2>
      </OverlayHeader>
      <div className="mx-4 flex h-[var(--control)] flex-none items-center gap-2 rounded-sm border border-line bg-surface-sunken px-2 text-ink-muted focus-within:border-line-strong">
        <Icon name="search" size={14} className="flex-none" />
        <input
          aria-label="Filter shortcuts"
          placeholder="Filter shortcuts"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          // biome-ignore lint/a11y/noAutofocus: typing to filter is the quickest way in
          autoFocus
          className="menu-search min-w-0 flex-1 bg-transparent text-body text-ink placeholder:text-ink-faint"
        />
      </div>
      <div className="min-h-0 overflow-y-auto px-4 pt-3 pb-4">
        {groups.length > 0 ? (
          <div className="grid gap-x-8 gap-y-5 sm:grid-cols-2">
            {groups.map((group) => (
              <section key={group.label} aria-label={group.label}>
                <h3 className="pb-1 text-label text-ink-muted">{group.label}</h3>
                <ul>
                  {group.entries.map((entry) => (
                    <li
                      key={entry.label}
                      className="-mx-2 flex h-[var(--control)] items-center gap-3 rounded-sm px-2 hover:bg-selection"
                    >
                      <span className="min-w-0 flex-1 truncate">{entry.label}</span>
                      <Kbd keys={entry.keys} />
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        ) : (
          <p className="py-8 text-center text-ink-muted">No shortcut matches “{query.trim()}”</p>
        )}
      </div>
    </Overlay>
  );
}
