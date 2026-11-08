import type { Project, Session } from "./types";

export interface SessionGroup {
  id: string;
  label: string;
  /** The project's main checkout, or the folder the group shows. */
  cwd?: string;
  /** The registered project this group shows. */
  project?: Project;
  /** For a folder that is not registered: the git repository it can be registered as. */
  checkout?: string;
  sessions: Session[];
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

const byRecent = (a: Session, b: Session) => b.updatedAt - a.updatedAt;

/**
 * Sidebar groups: open sessions (with a live process), then pinned sessions, then
 * every registered project and every other folder (git root, else folder) by most
 * recent activity, then registered projects with no sessions yet. A session appears
 * once, in the first group that claims it; archived sessions in none. A filter hides
 * empty projects.
 */
export function groupSessions(
  sessions: Session[],
  projects: Project[] = [],
  query = "",
): SessionGroup[] {
  const needle = query.trim().toLowerCase();
  const projectById = new Map(projects.map((p) => [p.id, p]));
  const matches = (s: Session) => {
    if (!needle) return true;
    const project = s.projectId === null ? undefined : projectById.get(s.projectId);
    return [s.title, s.projectRoot, project?.name ?? ""].some((text) =>
      text.toLowerCase().includes(needle),
    );
  };
  const visible = sessions.filter((s) => !s.archived && matches(s)).sort(byRecent);

  const open = visible.filter((s) => s.open);
  const pinned = visible.filter((s) => !s.open && s.pinned);
  const rest = visible.filter((s) => !s.open && !s.pinned);

  // Insertion order is most recent first, since `rest` is sorted.
  const places = new Map<string, SessionGroup>();
  for (const session of rest) {
    const project = session.projectId === null ? undefined : projectById.get(session.projectId);
    const id = project ? `project:${project.id}` : `folder:${session.projectRoot}`;
    let group = places.get(id);
    if (!group) {
      group = project
        ? { id, label: project.name, cwd: project.repo, project, sessions: [] }
        : {
            id,
            label: basename(session.projectRoot),
            cwd: session.projectRoot,
            checkout: session.checkout ?? undefined,
            sessions: [],
          };
      places.set(id, group);
    }
    group.sessions.push(session);
  }
  if (!needle) {
    for (const project of projects) {
      const id = `project:${project.id}`;
      if (!places.has(id)) {
        places.set(id, { id, label: project.name, cwd: project.repo, project, sessions: [] });
      }
    }
  }

  const groups: SessionGroup[] = [];
  if (open.length) groups.push({ id: "open", label: "Open", sessions: open });
  if (pinned.length) groups.push({ id: "pinned", label: "Pinned", sessions: pinned });
  groups.push(...places.values());
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
