import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { groupSessions, relativeTime } from "../grouping";
import { useStore } from "../store";
import type { SidebarSession } from "../types";

const COLLAPSED_PROJECT_LIMIT = 5;

async function pickFolder(): Promise<string | null> {
  const folder = await openDialog({ directory: true, title: "Start a session in…" });
  return typeof folder === "string" ? folder : null;
}

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const drafts = useStore((s) => s.drafts);
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
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button
          className="rounded border border-neutral-300 px-2 py-1 text-sm hover:bg-neutral-200 dark:border-neutral-700 dark:hover:bg-neutral-800"
          onClick={() => void startIn()}
          title="New session in a folder"
        >
          New
        </button>
      </div>
      <nav className="flex-1 overflow-y-auto pb-4">
        {groups.map((group) => (
          <Group key={group.id} {...group} onNew={group.cwd ? () => void startIn(group.cwd) : undefined} />
        ))}
        {groups.length === 0 && (
          <p className="px-3 text-sm text-neutral-500">No sessions{query ? " match" : " yet"}.</p>
        )}
      </nav>
    </aside>
  );
}

function Group(props: {
  id: string;
  label: string;
  cwd?: string;
  sessions: SidebarSession[];
  onNew?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const isProject = props.id.startsWith("project:");
  const shown =
    isProject && !expanded ? props.sessions.slice(0, COLLAPSED_PROJECT_LIMIT) : props.sessions;
  const hidden = props.sessions.length - shown.length;

  return (
    <section className="mt-3">
      <header className="group flex items-center justify-between px-3 text-xs font-semibold uppercase tracking-wide text-neutral-500">
        <span title={props.cwd}>{props.label}</span>
        {props.onNew && (
          <button
            className="invisible rounded px-1 text-neutral-500 hover:bg-neutral-200 group-hover:visible dark:hover:bg-neutral-800"
            onClick={props.onNew}
            title={`New session in ${props.cwd}`}
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
        <button className="px-3 text-xs text-neutral-500 hover:underline" onClick={() => setExpanded(true)}>
          {hidden} more
        </button>
      )}
    </section>
  );
}

function SessionItem({ session }: { session: SidebarSession }) {
  const active = useStore((s) => s.activeKey === session.key);
  const busy = useStore((s) => s.conversations[session.key]?.busy ?? false);
  const open = useStore((s) => s.open);
  const close = useStore((s) => s.close);
  const togglePin = useStore((s) => s.togglePin);

  return (
    <li
      className={`group flex cursor-default items-center gap-2 px-3 py-1.5 text-sm ${
        active ? "bg-neutral-200 dark:bg-neutral-800" : "hover:bg-neutral-100 dark:hover:bg-neutral-800/50"
      }`}
      onClick={() => void open(session)}
    >
      <span
        className={`h-1.5 w-1.5 shrink-0 rounded-full ${
          busy ? "animate-pulse bg-blue-500" : session.open ? "bg-green-500" : "bg-transparent"
        }`}
      />
      <span className="min-w-0 flex-1 truncate" title={`${session.title}\n${session.cwd}`}>
        {session.title}
      </span>
      <span className="text-xs text-neutral-400 group-hover:hidden">{relativeTime(session.updatedAt)}</span>
      <span className="hidden gap-1 group-hover:flex">
        <button
          className="text-xs text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100"
          onClick={(e) => {
            e.stopPropagation();
            void togglePin(session);
          }}
          title={session.pinned ? "Unpin" : "Pin"}
        >
          {session.pinned ? "Unpin" : "Pin"}
        </button>
        {session.open && (
          <button
            className="text-xs text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100"
            onClick={(e) => {
              e.stopPropagation();
              void close(session.key);
            }}
            title="Stop this session's agent process"
          >
            Stop
          </button>
        )}
      </span>
    </li>
  );
}
