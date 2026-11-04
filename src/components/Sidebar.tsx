import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { groupSessions, relativeTime, type SessionGroup } from "../grouping";
import { useStore } from "../store";
import type { SidebarSession } from "../types";

/** Sessions shown per project before "more". */
const COLLAPSED_PROJECT_LIMIT = 5;

const smallButton =
  "rounded px-1 text-xs text-neutral-500 hover:bg-neutral-200 hover:text-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100";

async function pickFolder(): Promise<string | null> {
  const folder = await openDialog({ directory: true, title: "Start a session in…" });
  return typeof folder === "string" ? folder : null;
}

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const drafts = useStore((s) => s.drafts);
  const listErrors = useStore((s) => s.listErrors);
  const create = useStore((s) => s.create);
  const [query, setQuery] = useState("");
  const groups = useMemo(
    () => groupSessions([...drafts, ...sessions], query),
    [sessions, drafts, query],
  );

  const startIn = async (cwd?: string) => {
    const folder = cwd ?? (await pickFolder());
    if (folder) await create(folder);
  };

  return (
    <aside className="flex w-72 shrink-0 flex-col border-r border-neutral-200 bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex items-center gap-2 p-3">
        <input
          className="min-w-0 flex-1 rounded border border-neutral-300 bg-white px-2 py-1 text-sm dark:border-neutral-700 dark:bg-neutral-950"
          placeholder="Filter sessions"
          aria-label="Filter sessions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button
          type="button"
          className="rounded border border-neutral-300 px-2 py-1 text-sm hover:bg-neutral-200 dark:border-neutral-700 dark:hover:bg-neutral-800"
          onClick={() => void startIn()}
          title="New session in a folder"
        >
          New
        </button>
      </div>
      {listErrors.map((error) => (
        <p key={error} className="px-3 text-xs text-red-700 dark:text-red-400">
          {error}
        </p>
      ))}
      <nav className="flex-1 overflow-y-auto pb-4">
        {groups.map((group) => (
          <Group
            key={group.id}
            group={group}
            onNew={group.cwd ? () => void startIn(group.cwd) : undefined}
          />
        ))}
        {groups.length === 0 && (
          <p className="px-3 text-sm text-neutral-500">No sessions{query ? " match" : " yet"}.</p>
        )}
      </nav>
    </aside>
  );
}

function Group({ group, onNew }: { group: SessionGroup; onNew?: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const collapsible = group.cwd !== undefined;
  const shown =
    collapsible && !expanded ? group.sessions.slice(0, COLLAPSED_PROJECT_LIMIT) : group.sessions;
  const hidden = group.sessions.length - shown.length;

  return (
    <section className="group/section mt-3">
      <header className="flex items-center justify-between px-3 text-xs font-semibold uppercase tracking-wide text-neutral-500">
        <span title={group.cwd}>{group.label}</span>
        {onNew && (
          <button
            type="button"
            className={`${smallButton} invisible group-hover/section:visible`}
            onClick={onNew}
            title={`New session in ${group.cwd}`}
          >
            +
          </button>
        )}
      </header>
      <ul>
        {shown.map((session) => (
          <SessionItem key={session.key} session={session} />
        ))}
      </ul>
      {hidden > 0 && (
        <button
          type="button"
          className="px-3 text-xs text-neutral-500 hover:underline"
          onClick={() => setExpanded(true)}
        >
          {hidden} more
        </button>
      )}
    </section>
  );
}

function SessionItem({ session }: { session: SidebarSession }) {
  const active = useStore((s) => s.activeKey === session.key);
  const busy = useStore((s) => s.conversations[session.key]?.busy ?? false);
  const waiting = useStore((s) => (s.conversations[session.key]?.questions.length ?? 0) > 0);
  const open = useStore((s) => s.open);
  const close = useStore((s) => s.close);
  const togglePin = useStore((s) => s.togglePin);

  const dot = waiting
    ? "bg-amber-500"
    : busy
      ? "animate-pulse bg-blue-500"
      : session.open
        ? "bg-green-500"
        : "bg-transparent";

  return (
    <li
      className={`group/item flex items-center gap-2 px-3 py-1.5 text-sm ${
        active
          ? "bg-neutral-200 dark:bg-neutral-800"
          : "hover:bg-neutral-100 dark:hover:bg-neutral-800/50"
      }`}
    >
      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dot}`} />
      <button
        type="button"
        className="min-w-0 flex-1 cursor-default truncate text-left"
        title={`${session.title}\n${session.cwd}`}
        aria-current={active ? "page" : undefined}
        onClick={() => void open(session)}
      >
        {session.title}
      </button>
      <span className="text-xs text-neutral-400 group-hover/item:hidden">
        {relativeTime(session.updatedAt)}
      </span>
      <span className="hidden gap-1 group-hover/item:flex">
        <button type="button" className={smallButton} onClick={() => void togglePin(session)}>
          {session.pinned ? "Unpin" : "Pin"}
        </button>
        {session.open && (
          <button
            type="button"
            className={smallButton}
            onClick={() => void close(session.key)}
            title="Stop this session's agent process"
          >
            Stop
          </button>
        )}
      </span>
    </li>
  );
}
