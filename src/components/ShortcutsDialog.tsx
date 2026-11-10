import { Dialog, Kbd } from "@panoptes/design-system";
import { useEffect, useRef } from "react";
import { SHORTCUT_GROUPS } from "../shortcuts";
import { useUi } from "../ui";

/** The ⌘/ cheatsheet: every shortcut the app responds to, in one dialog. */
export function ShortcutsDialog() {
  const close = useUi((s) => s.close);
  const first = useRef<HTMLDivElement>(null);

  // Focus inside the dialog so Escape closes it from the keyboard.
  useEffect(() => {
    first.current?.focus();
  }, []);

  return (
    <Dialog title="Keyboard shortcuts" width={480} onClose={close}>
      <div ref={first} tabIndex={-1} className="flex flex-col gap-5 focus:outline-none">
        {SHORTCUT_GROUPS.map((group) => (
          <section key={group.label}>
            <h3 className="pb-1 text-caption text-ink-muted">{group.label}</h3>
            <ul className="flex flex-col gap-1">
              {group.entries.map((entry) => (
                <li key={entry.label} className="flex items-center gap-3">
                  <span className="flex-1">{entry.label}</span>
                  <Kbd keys={entry.keys} />
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Dialog>
  );
}
