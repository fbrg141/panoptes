import type { SidebarSession } from "./types";

export interface SessionGroup {
  id: string;
  label: string;
  /** Folder new sessions in this group start in, when the group is a project. */
  cwd?: string;
  sessions: SidebarSession[];
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

const byRecent = (a: SidebarSession, b: SidebarSession) => b.updatedAt - a.updatedAt;

/**
 * Sidebar groups: open sessions (with a live process), then pinned sessions, then
 * every project (git root, else folder) by most recent activity. A session
 * appears once, in the first group that claims it.
 */
export function groupSessions(sessions: SidebarSession[], query = ""): SessionGroup[] {
  const needle = query.trim().toLowerCase();
  const visible = sessions
    .filter(
      (s) =>
        !needle ||
        s.title.toLowerCase().includes(needle) ||
        s.projectRoot.toLowerCase().includes(needle),
    )
    .sort(byRecent);

  const open = visible.filter((s) => s.open);
  const pinned = visible.filter((s) => !s.open && s.pinned);
  const rest = visible.filter((s) => !s.open && !s.pinned);

  const projects = new Map<string, SidebarSession[]>();
  for (const session of rest) {
    const list = projects.get(session.projectRoot) ?? [];
    list.push(session);
    projects.set(session.projectRoot, list);
  }

  const groups: SessionGroup[] = [];
  if (open.length) groups.push({ id: "open", label: "Open", sessions: open });
  if (pinned.length) groups.push({ id: "pinned", label: "Pinned", sessions: pinned });
  for (const [root, list] of projects) {
    groups.push({ id: `project:${root}`, label: basename(root), cwd: root, sessions: list });
  }
  return groups;
}

/** A past time in the design system's voice: "just now", "12m ago", "3d ago". */
export function relativeTime(ms: number, now = Date.now()): string {
  const minutes = Math.round((now - ms) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(ms).toLocaleDateString();
}
