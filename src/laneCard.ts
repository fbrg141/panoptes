import type { Session } from "./types";

/** The open root session, or the root most recently active when none is open. */
export function laneMainSession(sessions: Session[]): Session | undefined {
  const roots = sessions.filter((session) => session.parentLocator === null);
  const candidates = roots.length > 0 ? roots : sessions;
  return candidates.reduce<Session | undefined>((best, session) => {
    if (session.open) return session;
    if (best?.open) return best;
    return !best ||
      session.updatedAt > best.updatedAt ||
      (session.updatedAt === best.updatedAt && (session.id ?? 0) > (best.id ?? 0))
      ? session
      : best;
  }, undefined);
}

/**
 * The lane's live session that opening `session` would stop. Opening an
 * earlier record resumes it, which ends the current process, so the UI asks
 * first instead of doing it on a click.
 */
export function laneSessionToStop(session: Session, sessions: Session[]): Session | undefined {
  if (session.id === null || session.laneId === null || session.open) return undefined;
  return sessions.find(
    (item) => item.laneId === session.laneId && item.open && item.key !== session.key,
  );
}

/** Earlier root conversations available to /resume, newest created first. */
export function laneResumeSessions(sessions: Session[]): Session[] {
  const current = laneMainSession(sessions);
  return sessions
    .filter(
      (session) =>
        session.parentLocator === null && !session.archived && session.key !== current?.key,
    )
    .sort((a, b) => b.createdAt - a.createdAt || (b.id ?? 0) - (a.id ?? 0));
}
