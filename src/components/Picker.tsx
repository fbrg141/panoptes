import { Icon, Kbd } from "@panoptes/design-system";
import { type ComponentProps, Fragment, useEffect, useId, useMemo, useRef, useState } from "react";
import { filterPicker, initialPick, type PickerGroup, pickable } from "../picker";

type IconName = ComponentProps<typeof Icon>["name"];

/**
 * A button that opens a list to pick one value from, driven entirely from the
 * keyboard: the list opens on the current value, typing narrows it, arrows
 * move, Enter picks and Escape backs out without closing whatever the picker
 * sits in. The composer's model menu and the new lane and settings forms'
 * menus are all this one control, so they look and behave the same.
 */
export function Picker({
  label,
  title,
  icon,
  value,
  valueText,
  groups,
  onSelect,
  onOpen,
  searchable = true,
  placement = "down",
  variant = "chip",
  width = 320,
  emptyText = "Nothing to pick yet",
  disabled = false,
  returnFocus,
}: {
  /** What is being picked, for screen readers and the search placeholder. */
  label: string;
  title?: string;
  icon?: IconName;
  /** The trigger's content: the value in use now. */
  value: React.ReactNode;
  /** The value as words, when `value` is not plain text. */
  valueText?: string;
  groups: PickerGroup[];
  onSelect: (id: string) => void;
  onOpen?: () => void;
  /** Show a search field. Without one, typing still narrows the list. */
  searchable?: boolean;
  placement?: "up" | "down";
  /** A bordered chip in forms, or a borderless toolbar button. */
  variant?: "chip" | "ghost";
  width?: number;
  emptyText?: string;
  disabled?: boolean;
  /** Where focus goes after a pick or Escape; the trigger by default. */
  returnFocus?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();
  const visible = useMemo(() => filterPicker(groups, query), [groups, query]);
  const rows = useMemo(() => pickable(visible), [visible]);
  const total = useMemo(() => groups.reduce((n, group) => n + group.items.length, 0), [groups]);

  // Open on the current value, and close on a click anywhere else.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only when the menu opens or its items arrive
  useEffect(() => {
    if (!open) return;
    if (!query) setPicked(initialPick(groups));
    search.current?.focus();
  }, [open, total]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) dismiss(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  });

  // Keep the highlighted row in view as the arrows walk past the list's edge.
  useEffect(() => {
    if (open)
      list.current?.querySelector(`[data-row="${picked}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, picked]);

  const dismiss = (refocus: boolean) => {
    setOpen(false);
    setQuery("");
    if (refocus) (returnFocus ?? (() => trigger.current?.focus()))();
  };

  const choose = (itemId: string) => {
    onSelect(itemId);
    dismiss(true);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const last = rows.length - 1;
    const move = (to: number) => {
      e.preventDefault();
      if (rows.length > 0) setPicked(to);
    };
    switch (e.key) {
      case "ArrowDown":
        return move(picked >= last ? 0 : picked + 1);
      case "ArrowUp":
        return move(picked <= 0 ? last : picked - 1);
      case "Home":
        return searchable && query ? undefined : move(0);
      case "End":
        return searchable && query ? undefined : move(last);
      case "Enter":
        e.preventDefault();
        e.stopPropagation();
        if (rows[picked]) choose(rows[picked].id);
        return;
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        return dismiss(true);
      case "Tab":
        return dismiss(false);
    }
  };

  const listId = `${id}-list`;
  const rowId = (index: number) => `${id}-row-${index}`;
  let index = -1;

  return (
    <div ref={root} className="relative min-w-0">
      <button
        ref={trigger}
        type="button"
        title={title}
        aria-label={`${label}: ${valueText ?? (typeof value === "string" ? value : "")}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => {
          if (open) return dismiss(false);
          onOpen?.();
          setOpen(true);
        }}
        className={`picker-trigger is-${variant} flex h-[var(--control-sm)] max-w-full min-w-0 items-center gap-1.5 rounded-sm px-2 text-ink-muted transition-colors duration-[var(--duration-fast)] disabled:cursor-default disabled:text-ink-faint`}
      >
        {icon && <Icon name={icon} size={14} className="flex-none" />}
        <span className="min-w-0 truncate">{value}</span>
        <Icon name="chevron-down" size={12} className="flex-none opacity-70" />
      </button>

      {open && (
        <div
          className={`overlay-panel absolute left-0 z-30 flex flex-col overflow-hidden rounded-md bg-surface-raised shadow-[var(--shadow-pop)] ${
            placement === "up" ? "bottom-full mb-1.5" : "top-full mt-1.5"
          }`}
          style={{ width }}
        >
          <div
            className={
              searchable
                ? "flex h-[var(--control-lg)] flex-none items-center gap-2 border-b border-line px-3 text-ink-muted"
                : "sr-only"
            }
          >
            <Icon name="search" size={14} className="flex-none" />
            <input
              ref={search}
              role="combobox"
              aria-expanded
              aria-controls={listId}
              aria-activedescendant={rows.length > 0 ? rowId(picked) : undefined}
              aria-label={`Search ${label.toLowerCase()}s`}
              placeholder={`Search ${label.toLowerCase()}s`}
              value={query}
              className="menu-search min-w-0 flex-1 bg-transparent text-body text-ink placeholder:text-ink-faint"
              onChange={(event) => {
                setQuery(event.target.value);
                setPicked(0);
              }}
              onKeyDown={onKeyDown}
            />
          </div>
          <div
            ref={list}
            id={listId}
            role="listbox"
            aria-label={label}
            className="max-h-72 overflow-y-auto p-1"
          >
            {visible.map((group, g) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: groups are positional and unlabelled ones share no key
              <Fragment key={`${group.label ?? ""}${g}`}>
                {group.label && (
                  <div className="px-2 pt-2 pb-1 text-label text-ink-muted first:pt-1">
                    {group.label}
                  </div>
                )}
                {group.items.map((item) => {
                  if (item.disabled) {
                    return (
                      <div
                        key={item.id}
                        role="option"
                        tabIndex={-1}
                        aria-selected={false}
                        aria-disabled
                        className="flex h-[var(--control)] items-center gap-2 rounded-sm px-2 text-ink-faint"
                      >
                        <span className="w-3.5 flex-none" />
                        <span className="truncate">{item.label}</span>
                      </div>
                    );
                  }
                  index++;
                  const at = index;
                  const on = at === picked;
                  return (
                    // biome-ignore lint/a11y/useKeyWithClickEvents: the search field drives the list from the keyboard
                    <div
                      key={item.id}
                      id={rowId(at)}
                      data-row={at}
                      role="option"
                      tabIndex={-1}
                      aria-selected={on}
                      title={item.detail ? `${item.label} · ${item.detail}` : item.label}
                      className={`flex h-[var(--control)] cursor-default items-center gap-2 rounded-sm px-2 ${
                        on ? "bg-selection" : ""
                      }`}
                      onMouseDown={(event) => event.preventDefault()}
                      onMouseMove={() => at !== picked && setPicked(at)}
                      onClick={() => choose(item.id)}
                    >
                      {item.current ? (
                        <Icon name="check" size={14} className="flex-none text-ink" />
                      ) : (
                        <span className="w-3.5 flex-none" />
                      )}
                      <span
                        className={`truncate text-ink ${item.detail ? "max-w-[62%] flex-none" : "flex-1"}`}
                      >
                        {item.label}
                      </span>
                      {item.detail && (
                        <span className="text-code-sm min-w-0 flex-1 truncate text-right text-ink-muted">
                          {item.detail}
                        </span>
                      )}
                    </div>
                  );
                })}
              </Fragment>
            ))}
            {rows.length === 0 && (
              <p className="px-2 py-3 text-center text-ink-muted">
                {query.trim() ? `No match for “${query.trim()}”` : emptyText}
              </p>
            )}
          </div>
          {searchable && (
            <div className="flex h-[var(--control)] flex-none items-center gap-4 border-t border-line px-3 text-label font-normal text-ink-muted">
              <span className="inline-flex items-center gap-1.5">
                <Kbd keys={["↑", "↓"]} /> move
              </span>
              <span className="inline-flex items-center gap-1.5">
                <Kbd keys={["↵"]} /> choose
              </span>
              <span className="ml-auto inline-flex items-center gap-1.5">
                <Kbd keys={["Esc"]} /> back
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
