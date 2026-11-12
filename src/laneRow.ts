import type { Lane, LaneNavigation, LaneWork, LaneWriter } from "./types";

/**
 * What a lane row says about a lane with no session of its own. "No session"
 * hides three different situations, so each gets its own words and action:
 *
 * - `notStarted`: nothing changed or committed and nobody has written here.
 * - `hasWork`: the worktree holds work, made by another session or by hand;
 *   continue it here with a brief rather than a blank session.
 * - `beingWritten`: another session that has changed this lane is Running or
 *   Waiting, so starting a second writer is held back until it is Done.
 */
export type EmptyLane =
  | { kind: "notStarted" }
  | { kind: "hasWork" }
  | { kind: "beingWritten"; writer: LaneWriter };

/** Null when the lane has a session (or an archived one), or is archived. */
export function emptyLane(lane: Lane, navigation: LaneNavigation | undefined): EmptyLane | null {
  if (lane.state !== "active") return null;
  if ((navigation?.sessions.length ?? 0) > 0 || (navigation?.archivedSessions.length ?? 0) > 0) {
    return null;
  }
  const writers = navigation?.writers ?? [];
  const live = writers.find((writer) => writer.live);
  if (live) return { kind: "beingWritten", writer: live };
  if (writers.length > 0 || hasWork(navigation?.work ?? null)) return { kind: "hasWork" };
  return { kind: "notStarted" };
}

function hasWork(work: LaneWork | null): boolean {
  return work !== null && (work.changedFiles > 0 || (work.commitsAhead ?? 0) > 0);
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

/** The line naming who else changed a lane, from its most recent writer. */
export function writersLabel(writers: LaneWriter[], time: (at: number) => string): string | null {
  const [latest, ...rest] = writers;
  if (!latest) return null;
  const name = latest.title ? `“${latest.title}”` : "a removed session";
  const verb = latest.live
    ? latest.via === "edit"
      ? "Being edited by"
      : "Being changed by commands in"
    : latest.via === "edit"
      ? "Edited by"
      : "Changed by commands in";
  const others = rest.length > 0 ? ` and ${plural(rest.length, "other", "others")}` : "";
  return `${verb} ${name}${others} · ${time(latest.lastAt)}`;
}
