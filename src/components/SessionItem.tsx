import { Button, Icon, LoopMark, type MenuItem } from "@panoptes/design-system";
import { type KeyboardEvent, type ReactNode, useRef, useState, useSyncExternalStore } from "react";
import { relativeTime } from "../grouping";
import { isLive, primaryAction, sessionDetail, sessionRowLabel, statusWord } from "../sessionRow";
import { sessionMenuItems } from "../sidebarMenu";
import { useStore } from "../store";
import type { Session, SessionStatus } from "../types";
import { ContextMenu } from "./ContextMenu";
import { RowMenu } from "./RowMenu";

const MENU_SEPARATOR: MenuItem = { separator: true };

const WORD_TONE: Record<SessionStatus, string> = {
  running: "text-running",
  waiting: "font-medium text-waiting",
  failed: "font-medium text-error",
  done: "text-done",
  idle: "text-ink-muted",
};

/** One optical size for every status: a 16px box, each shape drawn to fill it evenly. */
function StatusGlyph({ status }: { status: SessionStatus }) {
  // Keyed by status so a change fades the new glyph in rather than swapping it.
  return (
    <span key={status} className="session-glyph flex h-4 w-6 items-center justify-center">
      {status === "running" && (
        <span className="flex text-running">
          <LoopMark size={20} tone="current" state="thinking" title="Running" />
        </span>
      )}
      {status === "waiting" && (
        <span className="size-2.5 rounded-full border-2 border-waiting" aria-hidden />
      )}
      {status === "failed" && <Icon name="circle-alert" size={16} className="text-error" />}
      {status === "done" && <Icon name="check" size={16} className="text-done" />}
      {status === "idle" && (
        <span className="size-2 rounded-full border-[1.5px] border-ink-muted" aria-hidden />
      )}
    </span>
  );
}

// One shared clock for every row's relative time, so "2m ago" keeps up
// without a timer per row.
const TICK_MS = 30_000;
const ticking = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;
function subscribeToClock(onTick: () => void) {
  ticking.add(onTick);
  timer ??= setInterval(() => {
    for (const tick of ticking) tick();
  }, TICK_MS);
  return () => {
    ticking.delete(onTick);
    if (ticking.size === 0) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}
const useClock = () =>
  useSyncExternalStore(subscribeToClock, () => Math.floor(Date.now() / TICK_MS) * TICK_MS);

/** Moves focus to the previous or next session row in the same list. */
function moveBetweenRows(from: HTMLElement, key: string) {
  const rows = Array.from(
    from.closest("nav")?.querySelectorAll<HTMLElement>("[data-session-hit]") ?? [],
  );
  const index = rows.indexOf(from);
  const target =
    key === "ArrowDown"
      ? rows[index + 1]
      : key === "ArrowUp"
        ? rows[index - 1]
        : key === "Home"
          ? rows[0]
          : rows[rows.length - 1];
  target?.focus();
}

/**
 * A session in the sidebar. The whole row is one button that opens it; Stop
 * (while it runs) and the actions menu sit over the meta column and fade in on
 * hover or focus, so nothing ever reflows. Arrows move between rows, Right
 * reaches the actions, and Shift+F10 or the menu key opens the menu.
 */
export function SessionItem({
  session,
  context,
  onAdopt,
  laneActions = false,
  nested = false,
  title: titleOverride,
  expand,
  extraMenuItems = [],
  onExtraMenuSelect,
  menuLabel,
  className = "",
  children,
}: {
  session: Session;
  context?: string;
  onAdopt?: () => void;
  laneActions?: boolean;
  nested?: boolean;
  /** A lane card keeps the lane's task as its visible title. */
  title?: string;
  /** A separate disclosure button; the session hit target always opens. */
  expand?: { expanded: boolean; label: string; onToggle: () => void };
  /** Lane actions share the main session card's context and row menus. */
  extraMenuItems?: MenuItem[];
  onExtraMenuSelect?: (id: string) => void;
  menuLabel?: string;
  className?: string;
  /** Extra sessions, disclosed separately beneath the main session card. */
  children?: ReactNode;
}) {
  const active = useStore((s) => s.activeKey === session.key);
  const status = useStore((s) => s.statuses[session.key] ?? "idle");
  const open = useStore((s) => s.open);
  const close = useStore((s) => s.close);
  const setArchived = useStore((s) => s.setArchived);
  const [menuOpen, setMenuOpen] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const hit = useRef<HTMLButtonElement>(null);
  const titleRef = useRef<HTMLSpanElement>(null);
  const actionsRef = useRef<HTMLDivElement>(null);
  const fromKeyboard = useRef(false);
  const menuFromKeyboard = useRef(false);

  const sessionItems = sessionMenuItems(session, {
    adoptable: onAdopt !== undefined,
    laneActions,
  });
  const items: MenuItem[] = [
    ...sessionItems,
    ...(sessionItems.length > 0 && extraMenuItems.length > 0 ? [MENU_SEPARATOR] : []),
    ...extraMenuItems,
  ];
  const primary = primaryAction(session, status);
  const hasActions = items.length > 0 || primary !== null;
  const word = statusWord(status);
  const detail = sessionDetail(session, context);
  const now = useClock();
  const time = relativeTime(session.updatedAt, Math.max(now, session.updatedAt));
  const label = sessionRowLabel({ title: titleOverride ?? session.title, status, detail, time });
  const title = titleOverride ?? session.title;
  const actionsLabel = menuLabel ?? `Actions for session ${session.title}`;

  const select = (id: string) => {
    if (id === "adopt") onAdopt?.();
    if (id === "stop") void close(session.key);
    if (id === "archive") void setArchived(session.key, !session.archived);
    onExtraMenuSelect?.(id);
  };

  // A title tooltip only when the title is actually cut off.
  const measure = () => {
    const el = titleRef.current;
    setTruncated(el !== null && el.scrollWidth > el.clientWidth);
  };

  const actionButtons = () =>
    Array.from(actionsRef.current?.querySelectorAll<HTMLElement>("button") ?? []);

  const onRowKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    const { key } = event;
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(key)) {
      event.preventDefault();
      moveBetweenRows(event.currentTarget, key);
    } else if (key === "ArrowRight" && hasActions) {
      event.preventDefault();
      actionButtons()[0]?.focus();
    } else if ((key === "F10" && event.shiftKey) || key === "ContextMenu") {
      if (items.length === 0) return;
      event.preventDefault();
      fromKeyboard.current = true;
      menuFromKeyboard.current = true;
      setMenuOpen(true);
    }
  };

  const onActionsKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (menuOpen || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")) return;
    const buttons = actionButtons();
    const index = buttons.indexOf(event.target as HTMLElement);
    if (index < 0) return;
    event.preventDefault();
    const next = index + (event.key === "ArrowRight" ? 1 : -1);
    (next < 0 ? hit.current : buttons[Math.min(next, buttons.length - 1)])?.focus();
  };

  return (
    <li className={`${nested ? "ml-2" : ""} ${className}`}>
      <ContextMenu label={actionsLabel} items={items} onSelect={select}>
        <div
          className="session-row group/session"
          data-selected={active || undefined}
          data-actions={hasActions || undefined}
          data-archived={session.archived || undefined}
        >
          {expand && (
            <Button
              variant="ghost"
              size="sm"
              icon={expand.expanded ? "chevron-down" : "chevron-right"}
              className="session-expand"
              aria-label={`${expand.expanded ? "Collapse" : "Expand"} ${expand.label}`}
              title={`${expand.expanded ? "Collapse" : "Expand"} ${expand.label}`}
              onClick={expand.onToggle}
            />
          )}
          <button
            ref={hit}
            type="button"
            data-session-hit
            className={`session-hit ${expand ? "session-hit-expandable" : ""}`}
            aria-current={active ? "page" : undefined}
            aria-label={label}
            aria-keyshortcuts={items.length > 0 ? "Shift+F10" : undefined}
            title={truncated ? title : undefined}
            onPointerEnter={measure}
            onFocus={measure}
            onClick={() => void open(session)}
            onKeyDown={onRowKey}
            onContextMenu={(event) => {
              // The menu key already opened the anchored menu; skip the pointer one.
              if (!fromKeyboard.current) return;
              fromKeyboard.current = false;
              event.preventDefault();
              event.stopPropagation();
            }}
          >
            <StatusGlyph status={status} />
            <span className="flex min-w-0 flex-col">
              <span
                ref={titleRef}
                className={`session-title truncate ${isLive(status) ? "text-body-strong" : "text-body"}`}
              >
                {title}
              </span>
              <span className="truncate text-xs text-ink-muted">
                {word && (
                  <>
                    <span className={`font-medium ${WORD_TONE[status]}`}>{word}</span>
                    <span aria-hidden> · </span>
                  </>
                )}
                {detail}
              </span>
            </span>
            <span className="session-meta">
              <span>{time}</span>
            </span>
          </button>
          {hasActions && (
            // biome-ignore lint/a11y/noStaticElementInteractions: arrow keys move between the row's buttons
            <div ref={actionsRef} className="session-actions" onKeyDown={onActionsKey}>
              {primary && (
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  tabIndex={-1}
                  aria-label={`${primary.label} ${session.title}`}
                  title={primary.label}
                  onClick={() => select(primary.id)}
                >
                  {/* Filled, so it reads as Stop and never as an empty checkbox. */}
                  <span className="block size-2.5 rounded-xs bg-current" aria-hidden />
                </Button>
              )}
              {items.length > 0 && (
                <RowMenu
                  label={actionsLabel}
                  items={items}
                  onSelect={select}
                  triggerClassName=""
                  triggerTabIndex={-1}
                  open={menuOpen}
                  onOpenChange={(shown) => {
                    setMenuOpen(shown);
                    // A menu opened from the row hands focus back to the row.
                    if (!shown && menuFromKeyboard.current) {
                      menuFromKeyboard.current = false;
                      requestAnimationFrame(() => hit.current?.focus());
                    }
                  }}
                />
              )}
            </div>
          )}
        </div>
      </ContextMenu>
      {children}
    </li>
  );
}
