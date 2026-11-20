import { laneSessionToStop } from "./laneCard";
import { useStore } from "./store";
import type { Session } from "./types";
import { useUi } from "./ui";

/**
 * Shows a session and opens it, unless that would stop the lane's live
 * session: then it asks first, through the resume confirmation.
 */
export async function requestOpen(session: Session, paneId?: string): Promise<void> {
  const state = useStore.getState();
  if (
    session.laneId !== null &&
    session.id !== null &&
    laneSessionToStop(session, state.sessions)
  ) {
    useUi.getState().open({ type: "resumeSession", laneId: session.laneId, sessionId: session.id });
    return;
  }
  useUi.getState().showSession(session.key, paneId);
  await state.open(session);
}
