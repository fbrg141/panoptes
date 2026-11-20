import { CommandPalette } from "@panoptes/design-system";
import { useMemo, useState } from "react";
import { laneResumeSessions } from "../laneCard";
import { useStore } from "../store";
import { useUi } from "../ui";
import { ModalLayer } from "./Overlay";

/** A lane-owned history picker; it never scans the harness's shared cwd. */
export function ResumeSessionDialog({ laneId }: { laneId: number }) {
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

  const select = async (id: string) => {
    const sessionId = Number(id.slice("session:".length));
    if (!Number.isInteger(sessionId) || busy) return;
    setBusy(true);
    if (await resume(laneId, sessionId)) close();
    else setBusy(false);
  };

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
        onSelect={(id) => void select(id)}
      />
    </ModalLayer>
  );
}
