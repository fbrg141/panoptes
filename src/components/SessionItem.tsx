import { Button, Icon, LoopMark } from "@panoptes/design-system";
import { type KeyboardEvent, useRef, useState, useSyncExternalStore } from "react";
import { relativeTime } from "../grouping";
import {
  isLive,
  lockLabel,
  primaryAction,
  sessionDetail,
  sessionRowLabel,
  statusWord,
} from "../sessionRow";
import { sessionMenuItems } from "../sidebarMenu";
import { useStore } from "../store";
import type { Session, SessionStatus } from "../types";
import { ContextMenu } from "./ContextMenu";
import { RowMenu } from "./RowMenu";

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
}: {
  session: Session;
  context?: string;
  onAdopt?: () => void;
  laneActions?: boolean;
  nested?: boolean;
}) {
  const active = useStore((s) => s.activeKey === session.key);
  const status = useStore((s) => s.statuses[session.key] ?? "idle");
  const lock = useStore((s) => s.locks[session.key]);
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

  const items = sessionMenuItems(session, { adoptable: onAdopt !== undefined, laneActions });
  const primary = primaryAction(session, status);
  const hasActions = items.length > 0 || primary !== null;
  const word = statusWord(status);
  const detail = sessionDetail(session, context);
  const now = useClock();
  const time = relativeTime(session.updatedAt, Math.max(now, session.updatedAt));
  const lockText = lockLabel(lock);
  const label = sessionRowLabel({ title: session.title, status, lock, detail, time });

  const select = (id: string) => {
    if (id === "adopt") onAdopt?.();
    if (id === "stop") void close(session.key);
    if (id === "archive") void setArchived(session.key, !session.archived);
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
    <li className={nested ? "ml-2" : ""}>
      <ContextMenu label={`Actions for session ${session.title}`} items={items} onSelect={select}>
        <div
          className="session-row group/session"
          data-selected={active || undefined}
          data-actions={hasActions || undefined}
          data-archived={session.archived || undefined}
        >
          <button
            ref={hit}
            type="button"
            data-session-hit
            className="session-hit"
            aria-current={active ? "page" : undefined}
            aria-label={label}
            aria-keyshortcuts={items.length > 0 ? "Shift+F10" : undefined}
            title={truncated ? session.title : undefined}
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
                {session.title}
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
              {lockText && (
                <span className="inline-flex items-center gap-1" title={lockText}>
                  <Icon name="lock" size={12} />
                  {lock?.queued && !lock.holder && "queued"}
                </span>
              )}
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
                  label={`Actions for session ${session.title}`}
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
    </li>
  );
}
