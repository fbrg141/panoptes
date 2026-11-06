import { AgentRow, Button, Input, LoopMark } from "@panoptes/design-system";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { groupSessions, relativeTime, type SessionGroup } from "../grouping";
import { useStore } from "../store";
import type { SidebarSession } from "../types";

/** Sessions shown per project before "more". */
const COLLAPSED_PROJECT_LIMIT = 5;

async function pickFolder(): Promise<string | null> {
  const folder = await openDialog({ directory: true, title: "Start a session in…" });
  return typeof folder === "string" ? folder : null;
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

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
    <aside className="flex w-72 shrink-0 flex-col border-r border-line bg-surface">
      <header className="flex h-[var(--titlebar)] items-center gap-2 border-b border-line px-3">
        <LoopMark size={20} />
        <span className="text-wordmark flex-1">Panoptes</span>
        <Button
          variant="ghost"
          size="sm"
          icon="plus"
          aria-label="New session"
          title="New session in a folder"
          onClick={() => void startIn()}
        />
      </header>
      <div className="p-3">
        <Input
          icon="search"
          placeholder="Filter sessions"
          aria-label="Filter sessions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      {listErrors.map((error) => (
        <p key={error} className="px-3 pb-2 text-error">
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
          <p className="px-3 text-ink-muted">
            {query ? "No sessions match." : "No sessions yet. Start one with the + button."}
          </p>
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
    <section className="group/section mt-2">
      <header className="flex h-7 items-center justify-between px-3 text-ink-muted">
        <h2 className="text-caption" title={group.cwd}>
          {group.label}
        </h2>
        {onNew && (
          <Button
            variant="ghost"
            size="sm"
            icon="plus"
            aria-label={`New session in ${group.label}`}
            title={`New session in ${group.cwd}`}
            className="invisible group-hover/section:visible"
            onClick={onNew}
          />
        )}
      </header>
      <ul>
        {shown.map((session) => (
          <SessionItem key={session.key} session={session} />
        ))}
      </ul>
      {hidden > 0 && (
        <Button variant="ghost" size="sm" className="ml-2" onClick={() => setExpanded(true)}>
          {hidden} more
        </Button>
      )}
    </section>
  );
}

function SessionItem({ session }: { session: SidebarSession }) {
  const active = useStore((s) => s.activeKey === session.key);
  const status = useStore((s) => s.statuses[session.key] ?? "idle");
  const open = useStore((s) => s.open);
  const close = useStore((s) => s.close);
  const togglePin = useStore((s) => s.togglePin);

  return (
    <li className="group/item relative">
      <AgentRow
        name={session.title}
        task={basename(session.cwd)}
        status={status}
        time={relativeTime(session.updatedAt)}
        selected={active}
        onClick={() => void open(session)}
      />
      <span className="absolute top-1 right-2 hidden gap-1 rounded-sm bg-surface group-hover/item:flex">
        <Button variant="ghost" size="sm" onClick={() => void togglePin(session)}>
          {session.pinned ? "Unpin" : "Pin"}
        </Button>
        {session.open && (
          <Button
            variant="ghost"
            size="sm"
            title="Stop this session's agent process"
            onClick={() => void close(session.key)}
          >
            Stop
          </Button>
        )}
      </span>
    </li>
  );
}
