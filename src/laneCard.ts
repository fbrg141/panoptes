import type { Session } from "./types";

/**
 * The session a lane card opens: the lane's first root session, by creation.
 * It must not follow activity, or one click would open a different session
 * each time another one ran. A lane with no root session falls back to its
 * oldest session.
 */
export function laneMainSession(sessions: Session[]): Session | undefined {
  const oldest = (list: Session[]) =>
    list.reduce<Session | undefined>(
      (best, session) =>
        !best ||
        session.createdAt < best.createdAt ||
        (session.createdAt === best.createdAt && (session.id ?? 0) < (best.id ?? 0))
          ? session
          : best,
      undefined,
    );
  return oldest(sessions.filter((session) => session.parentLocator === null)) ?? oldest(sessions);
}
