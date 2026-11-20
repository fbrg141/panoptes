import { Button, CommandPalette } from "@panoptes/design-system";
import { useMemo, useState } from "react";
import { laneResumeSessions } from "../laneCard";
import { useStore } from "../store";
import { useUi } from "../ui";
import { ModalDialog, ModalLayer } from "./Overlay";

/**
 * A lane-owned history picker; it never scans the harness's shared cwd.
 * With `sessionId`, it only confirms resuming that record, since doing so
 * stops the lane's live session.
 */
export function ResumeSessionDialog({ laneId, sessionId }: { laneId: number; sessionId?: number }) {
  const sessions = useStore((s) => s.sessions);
  const lane = useStore((s) => s.lanes.find((item) => item.id === laneId));
  const resume = useStore((s) => s.resumeLaneSession);
  const close = useUi((s) => s.close);
  const [busy, setBusy] = useState(false);
  const groups = useMemo(
    () => [
      {
        label: "Earlier sessions",
        items: laneResumeSessions(sessions.filter((session) => session.laneId === laneId)).map(
          (session) => ({
            id: `session:${session.id}`,
            label: session.title,
            hint: `${session.messageCount} messages`,
            icon: "history",
          }),
        ),
      },
    ],
    [sessions, laneId],
  );

  const resumeId = async (id: number) => {
    if (busy) return;
    setBusy(true);
    if (await resume(laneId, id)) close();
    else setBusy(false);
  };

  const select = (id: string) => {
    const picked = Number(id.slice("session:".length));
    if (Number.isInteger(picked)) void resumeId(picked);
  };

  if (sessionId !== undefined) {
    const target = sessions.find((session) => session.id === sessionId);
    return (
      <ModalDialog
        title={`Resume ${target?.title ?? "this session"}?`}
        width={440}
        onClose={busy ? undefined : close}
        actions={
          <>
            <Button variant="secondary" disabled={busy} onClick={close}>
              Cancel
            </Button>
            <Button disabled={busy} onClick={() => void resumeId(sessionId)}>
              Resume
            </Button>
          </>
        }
      >
        <p>
          Only one session runs in a lane. Resuming this one stops the current session; its
          conversation stays in the lane's history.
        </p>
      </ModalDialog>
    );
  }

  return (
    <ModalLayer
      label={`Resume session in ${lane?.task ?? "lane"}`}
      onClose={busy ? undefined : close}
      className="max-w-full"
    >
      <CommandPalette
        className="overlay-panel"
        groups={groups}
        placeholder="Resume an earlier session"
        autoFocus
        onSelect={select}
      />
    </ModalLayer>
  );
}
