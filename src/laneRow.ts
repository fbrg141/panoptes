import type { Lane, LaneNavigation, LaneWork } from "./types";

/**
 * What a lane row offers when the lane has no session of its own: Start
 * session, or nothing while another session that edited the lane is still
 * Running or Waiting, since that would make two writers.
 */
export type EmptyLane = { kind: "startable" } | { kind: "busy"; writer: string };

/** Null when the lane has a session (or an archived one), or is archived. */
export function emptyLane(lane: Lane, navigation: LaneNavigation | undefined): EmptyLane | null {
  if (lane.state !== "active") return null;
  if ((navigation?.sessions.length ?? 0) > 0 || (navigation?.archivedSessions.length ?? 0) > 0) {
    return null;
  }
  const writer = navigation?.busyWriter;
  return writer ? { kind: "busy", writer: writer.title } : { kind: "startable" };
}

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** "6 files changed · 2 commits ahead", "No changes yet", or null if unknown. */
export function workLabel(work: LaneWork | null): string | null {
  if (!work) return null;
  const parts: string[] = [];
  if (work.changedFiles > 0) parts.push(`${plural(work.changedFiles, "file", "files")} changed`);
  if ((work.commitsAhead ?? 0) > 0) {
    parts.push(`${plural(work.commitsAhead ?? 0, "commit", "commits")} ahead`);
  }
  return parts.length > 0 ? parts.join(" · ") : "No changes yet";
}
