import { Button, Icon, Input, LoopMark, StatusPill } from "@panoptes/design-system";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { SESSION_DRAG_TYPE, type SplitDirection } from "../panes";
import { useStore } from "../store";
import { plainField } from "../textFields";
import { buildTurns, looseApprovals } from "../turns";
import type { Session } from "../types";
import { ApprovalCard } from "./ApprovalCard";
import { Composer } from "./Composer";
import { ComposerRunningIndicator } from "./ComposerRunningIndicator";
import { QuestionCard } from "./QuestionCard";
import { TurnView } from "./Turn";

/** Turns rendered at first; earlier ones load on request. */
const PAGE = 50;
/** Within this many px of the end, the transcript counts as at the bottom. */
const PINNED = 24;

export function ChatPane({
  sessionKey,
  focused = true,
  onSplit,
  onClose,
  canClose = false,
}: {
  sessionKey: string;
  focused?: boolean;
  onSplit?: (direction: SplitDirection) => void;
  onClose?: () => void;
  canClose?: boolean;
}) {
  const conversation = useStore((s) => s.conversations[sessionKey]);
  const status = useStore((s) => s.statuses[sessionKey]) ?? "idle";
  const sending = useStore((s) => s.sending[sessionKey] ?? false);
  const session = useStore((s) => s.sessions.find((x) => x.key === sessionKey));
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  // Follow new output while the user is at the bottom; stop once they scroll up.
  const pinned = useRef(true);
  // Distance from the end to restore after earlier turns are prepended.
  const anchor = useRef<number | null>(null);
  const [behind, setBehind] = useState(false);
  const [shown, setShown] = useState(PAGE);

  const turns = useMemo(() => (conversation ? buildTurns(conversation) : []), [conversation]);

  const toBottom = useCallback(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
    pinned.current = true;
    setBehind(false);
  }, []);

  // A different session starts from its latest page, at the end.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs per session
  useLayoutEffect(() => {
    setShown(PAGE);
    toBottom();
  }, [sessionKey]);

  // Keep the end in view as content grows (streaming text, a row opening),
  // and keep the reading position when earlier turns are prepended.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the transcript mounts once a session is active
  useEffect(() => {
    const el = scroller.current;
    if (!el || !content.current) return;
    const observer = new ResizeObserver(() => {
      if (anchor.current !== null) {
        el.scrollTop = el.scrollHeight - anchor.current;
        anchor.current = null;
      } else if (pinned.current) {
        el.scrollTop = el.scrollHeight;
      }
    });
    observer.observe(content.current);
    return () => observer.disconnect();
  }, [sessionKey]);

  // New output while scrolled up shows the jump button.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs per update
  useEffect(() => {
    if (!pinned.current) setBehind(true);
  }, [conversation?.messages]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const editing = e.target instanceof HTMLElement && e.target.closest("input, textarea");
      if (focused && e.metaKey && e.key === "ArrowDown" && !editing) {
        e.preventDefault();
        toBottom();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toBottom, focused]);

  const hidden = Math.max(0, turns.length - shown);

  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the header is a drag handle; splitting remains keyboard accessible */}
      <header
        className={`flex h-[var(--titlebar)] flex-none items-center gap-2 border-b px-3 ${
          focused ? "border-line-strong" : "border-line"
        }`}
        draggable
        onDragStart={(event) => {
          event.dataTransfer.setData(SESSION_DRAG_TYPE, sessionKey);
          event.dataTransfer.effectAllowed = "move";
        }}
      >
        {status === "running" && <LoopMark size={20} state="thinking" tone="current" />}
        <div className="min-w-0 flex-1">
          {session ? (
            <SessionTitle session={session} />
          ) : (
            <div className="text-body-strong truncate">Session</div>
          )}
        </div>
        <span
          className="text-code-sm hidden truncate text-ink-muted 2xl:block"
          title={session?.cwd}
        >
          {session?.cwd}
        </span>
        {status !== "idle" && <StatusPill status={status} />}
        {onSplit && (
          <div className="flex flex-none items-center">
            <Button
              variant="ghost"
              size="sm"
              icon="panel-left"
              iconOnly
              aria-label="Split vertically"
              title="Split vertically (⌘/Ctrl+D)"
              onClick={() => onSplit("vertical")}
            />
            <Button
              variant="ghost"
              size="sm"
              icon="panel-left"
              iconOnly
              className="rotate-90"
              aria-label="Split horizontally"
              title="Split horizontally (⌘/Ctrl+Shift+D)"
              onClick={() => onSplit("horizontal")}
            />
            {canClose && (
              <Button
                variant="ghost"
                size="sm"
                icon="x"
                iconOnly
                aria-label="Close pane"
                title="Close pane"
                onClick={onClose}
              />
            )}
          </div>
        )}
      </header>
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div
          ref={scroller}
          className="flex-1 overflow-y-auto px-4 py-6"
          onScroll={(e) => {
            const el = e.currentTarget;
            pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < PINNED;
            if (pinned.current) setBehind(false);
          }}
        >
          <div ref={content} className="mx-auto flex max-w-3xl flex-col gap-6">
            {!conversation && (
              <div className="flex items-center gap-3 text-ink-muted">
                <LoopMark size={20} state="loading" tone="current" />
                Starting the agent…
              </div>
            )}
            {hidden > 0 && (
              <Button
                variant="ghost"
                size="sm"
                className="self-center"
                onClick={() => {
                  const el = scroller.current;
                  if (el) anchor.current = el.scrollHeight - el.scrollTop;
                  setShown((n) => n + PAGE);
                }}
              >
                Show {Math.min(PAGE, hidden)} earlier turns ({hidden} hidden)
              </Button>
            )}
            {turns.slice(hidden).map((turn) => (
              <TurnView key={turn.key} turn={turn} />
            ))}
          </div>
        </div>
        {behind && (
          <div className="pointer-events-none absolute inset-x-0 bottom-2 flex justify-center">
            <Button
              size="sm"
              icon="chevron-down"
              kbd={["⌘", "↓"]}
              className="pointer-events-auto bg-surface-raised shadow-[var(--shadow-pop)]"
              onClick={toBottom}
            >
              Jump to latest
            </Button>
          </div>
        )}
      </div>
      {conversation?.notices.map((notice, i) => (
        <div
          // Notices are append-only and capped, so the index is stable.
          // biome-ignore lint/suspicious/noArrayIndexKey: see above
          key={i}
          className={`flex items-center gap-2 border-t border-line px-4 py-1 ${
            notice.level === "error"
              ? "bg-error-soft text-error"
              : notice.level === "warning"
                ? "bg-waiting-soft text-waiting"
                : "text-ink-muted"
          }`}
        >
          {notice.level !== "info" && <Icon name="circle-alert" size={14} />}
          {notice.text}
        </div>
      ))}
      {conversation?.exited && (
        <div
          className="flex items-center gap-2 border-t border-line bg-error-soft px-4 py-1 text-error"
          title={conversation.exited}
        >
          <Icon name="circle-alert" size={14} />
          The agent process stopped. Send a message to restart it.
        </div>
      )}
      <div className="px-4 pb-4 pt-2">
        {conversation?.questions.map((q) => (
          <QuestionCard key={q.id} question={q} sessionKey={sessionKey} />
        ))}
        {conversation &&
          looseApprovals(conversation).map((a) => (
            <div key={a.id} className="mx-auto mb-3 w-full max-w-3xl">
              <ApprovalCard approval={a} sessionKey={sessionKey} />
            </div>
          ))}
        <ComposerRunningIndicator status={status} />
        <Composer sessionKey={sessionKey} busy={conversation?.busy ?? false} sending={sending} />
      </div>
    </main>
  );
}

/* The session's name in the header: click it to rename, empty falls back. */
function SessionTitle({ session }: { session: Session }) {
  const rename = useStore((s) => s.rename);
  const [draft, setDraft] = useState<string | null>(null);

  const commit = () => {
    if (draft === null) return;
    setDraft(null);
    void rename(session.key, draft);
  };

  // Only a session a lane has adopted has a Panoptes record to rename.
  if (session.id === null) {
    return (
      <div className="max-w-full truncate text-body-strong" title={session.title}>
        {session.title}
      </div>
    );
  }

  if (draft !== null) {
    return (
      <Input
        className="max-w-md"
        aria-label="Session name"
        {...plainField}
        value={draft}
        autoFocus
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
          if (event.key === "Escape") setDraft(null);
        }}
        onBlur={commit}
      />
    );
  }

  return (
    <button
      type="button"
      className="block max-w-full truncate text-body-strong underline-offset-4 hover:underline"
      title="Rename session"
      onClick={() => setDraft(session.title)}
    >
      {session.title}
    </button>
  );
}
