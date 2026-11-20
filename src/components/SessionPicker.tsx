import { CommandPalette } from "@panoptes/design-system";
import { useMemo } from "react";
import { requestOpen } from "../openSession";
import { recentSessions } from "../palette";
import { useStore } from "../store";
import type { Project, Session } from "../types";
import { useUi } from "../ui";
import { ModalLayer } from "./Overlay";

/** New work is the first choice; recent sessions follow it. */
export function sessionPickerGroups(sessions: Session[], projects: Project[]) {
  return [
    {
      label: "Actions",
      items: [
        {
          id: "new",
          label: "New session",
          hint: projects[0] ? `Start a lane in ${projects[0].name}` : "Add a project first",
          icon: "plus",
        },
      ],
    },
    {
      label: "Sessions",
      items: recentSessions(sessions).map((session) => ({
        id: `session:${session.key}`,
        label: session.title,
        hint: session.cwd,
        icon: "session",
      })),
    },
  ];
}

/** A command palette that fills one empty or split pane. */
export function SessionPicker({ paneId }: { paneId: string }) {
  const sessions = useStore((s) => s.sessions);
  const projects = useStore((s) => s.projects);
  const close = useUi((s) => s.close);
  const show = useUi((s) => s.open);
  const groups = useMemo(() => sessionPickerGroups(sessions, projects), [sessions, projects]);

  const select = async (id: string) => {
    close();
    if (id === "new") {
      show({ type: "newLane", projectId: projects[0]?.id ?? 0 });
      return;
    }
    if (!id.startsWith("session:")) return;
    const key = id.slice("session:".length);
    const session = sessions.find((item) => item.key === key);
    if (!session) return;
    await requestOpen(session, paneId);
  };

  return (
    <ModalLayer label="Choose a session" onClose={close} className="max-w-full">
      <CommandPalette
        className="overlay-panel"
        groups={groups}
        placeholder="Select a session for this pane"
        autoFocus
        onSelect={(id) => void select(id)}
      />
    </ModalLayer>
  );
}
