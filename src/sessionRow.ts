import type { Session, SessionStatus } from "./types";

/** The status words the guide allows; idle has none and recedes instead. */
export const STATUS_WORDS: Record<Exclude<SessionStatus, "idle">, string> = {
  running: "Running",
  waiting: "Waiting",
  failed: "Failed",
  done: "Done",
};

export const statusWord = (status: SessionStatus): string | null =>
  status === "idle" ? null : STATUS_WORDS[status];

/**
 * Whether the row should carry weight: an agent at work or one that needs
 * you. Done and idle rows recede to a regular-weight title.
 */
export const isLive = (status: SessionStatus): boolean =>
  status === "running" || status === "waiting" || status === "failed";

/**
 * The second line of a session row. Nested under a lane, the lane already
 * names the project and branch, so the row says only what is its own.
 */
export function sessionDetail(
  session: Pick<Session, "archived" | "messageCount">,
  context: string | undefined,
): string {
  const own =
    context ?? (session.messageCount === 1 ? "1 message" : `${session.messageCount} messages`);
  return session.archived ? `Archived · ${own}` : own;
}

/**
 * The row's accessible name: the title first (so voice control matches what
 * is shown), then everything the glyphs and colours say.
 */
export function sessionRowLabel({
  title,
  status,
  detail,
  time,
}: {
  title: string;
  status: SessionStatus;
  detail: string;
  time: string;
}): string {
  return [title, statusWord(status), detail, `updated ${time}`].filter(Boolean).join(", ");
}

/** The one action worth a button on the row itself; the rest stay in its menu. */
export function primaryAction(
  session: Pick<Session, "open">,
  status: SessionStatus,
): { id: "stop"; label: string } | null {
  if (session.open && status === "running") return { id: "stop", label: "Stop" };
  return null;
}
