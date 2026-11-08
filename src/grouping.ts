import type { Lane, Project, Session } from "./types";

/**
 * Search the backend's flat All sessions list without changing its ordering or
 * membership. Lane and project names are searchable metadata, not grouping.
 */
export function filterSessions(
  sessions: Session[],
  projects: Project[],
  lanes: Lane[],
  query = "",
): Session[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return sessions;
  const projectById = new Map(projects.map((project) => [project.id, project]));
  const laneById = new Map(lanes.map((lane) => [lane.id, lane]));
  return sessions.filter((session) => {
    const project = session.projectId === null ? undefined : projectById.get(session.projectId);
    const lane = session.laneId === null ? undefined : laneById.get(session.laneId);
    return [
      session.title,
      session.cwd,
      project?.name ?? "",
      lane?.task ?? "",
      ...session.tags,
    ].some((text) => text.toLowerCase().includes(needle));
  });
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
