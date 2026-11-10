import { Dialog, Input, Kbd } from "@panoptes/design-system";
import { useMemo, useState } from "react";
import { filterShortcuts, SHORTCUT_GROUPS } from "../shortcuts";
import { useUi } from "../ui";

/**
 * The ⌘/ cheatsheet: every shortcut the app responds to, in one dialog, with
 * a filter on top so a long list stays one keystroke from the row you want.
 */
export function ShortcutsDialog() {
  const close = useUi((s) => s.close);
  const [query, setQuery] = useState("");
  const groups = useMemo(() => filterShortcuts(SHORTCUT_GROUPS, query), [query]);

  return (
    <Dialog className="overlay-panel" title="Keyboard shortcuts" width={520} onClose={close}>
      <div className="flex flex-col gap-4">
        <Input
          icon="search"
          placeholder="Filter shortcuts"
          aria-label="Filter shortcuts"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          autoFocus
        />
        <div className="-mx-4 max-h-[min(60vh,440px)] overflow-y-auto px-4">
          {groups.map((group, index) => (
            <section key={group.label} className={index > 0 ? "pt-4" : ""}>
              <h3 className="pb-1 text-caption text-ink-muted">{group.label}</h3>
              <ul>
                {group.entries.map((entry) => (
                  <li
                    key={entry.label}
                    className="flex h-8 items-center gap-3 border-b border-line text-ink last:border-b-0"
                  >
                    <span className="min-w-0 flex-1 truncate">{entry.label}</span>
                    <Kbd keys={entry.keys} />
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {groups.length === 0 && (
            <p className="py-6 text-center text-ink-muted">No shortcuts match “{query.trim()}”.</p>
          )}
        </div>
      </div>
    </Dialog>
  );
}
