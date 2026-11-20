import { CommandPalette } from "@panoptes/design-system";
import { useEffect, useMemo, useRef } from "react";
import { requestOpen } from "../openSession";
import { paletteGroups, paletteTarget } from "../palette";
import { pickProjectFolder } from "../pickProject";
import { useStore } from "../store";
import { useUi } from "../ui";
import { ModalLayer } from "./Overlay";

/**
 * The ⌘P overlay: one search box over projects, lanes, sessions and a few
 * actions. The sidebar keeps its trees; this is how you jump to any of them.
 */
export function SearchPalette() {
  const sessions = useStore((s) => s.sessions);
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const activeKey = useStore((s) => s.activeKey);
  const create = useStore((s) => s.create);
  const newLaneSession = useStore((s) => s.newLaneSession);
  const showLogs = useStore((s) => s.showLogs);
  const close = useUi((s) => s.close);
  const show = useUi((s) => s.open);
  const groups = useMemo(
    () => paletteGroups(sessions, projects, lanes, activeKey),
    [sessions, projects, lanes, activeKey],
  );
  const list = useRef<HTMLDivElement>(null);

  // The palette moves its highlight but never scrolls to it; keep the
  // highlighted row in view as the arrows walk past the list's edge.
  useEffect(() => {
    const root = list.current;
    if (!root) return;
    const follow = new MutationObserver(() => {
      root
        .querySelector<HTMLElement>('[role="option"][aria-selected="true"]')
        ?.scrollIntoView({ block: "nearest" });
    });
    follow.observe(root, { subtree: true, attributeFilter: ["aria-selected"] });
    return () => follow.disconnect();
  }, []);

  const select = async (id: string) => {
    const target = paletteTarget(id, sessions, projects, lanes);
    close();
    if (!target) return;
    switch (target.kind) {
      case "session":
        await requestOpen(target.session);
        break;
      case "lane": {
        const session = sessions.find((item) => item.laneId === target.lane.id);
        if (session) {
          await requestOpen(session);
        } else await create(target.lane.id);
        break;
      }
      case "project":
        show({ type: "newLane", projectId: target.project.id });
        break;
      case "newSession":
        show({ type: "newLane", projectId: projects[0]?.id ?? 0 });
        break;
      case "newLaneSession": {
        const laneId = sessions.find((session) => session.key === activeKey)?.laneId;
        if (laneId !== null && laneId !== undefined) await newLaneSession(laneId);
        break;
      }
      case "resumeSession": {
        const laneId = sessions.find((session) => session.key === activeKey)?.laneId;
        if (laneId !== null && laneId !== undefined) show({ type: "resumeSession", laneId });
        break;
      }
      case "newProject":
        await pickProjectFolder();
        break;
      case "shortcuts":
        show({ type: "shortcuts" });
        break;
      case "showLogs":
        await showLogs();
        break;
    }
  };

  return (
    // The palette ships without its own scrim; this is the Dialog one, with
    // the panel pushed toward the top like a command palette.
    <ModalLayer label="Search" onClose={close} className="max-w-full">
      <div ref={list} className="contents">
        <CommandPalette
          className="overlay-panel"
          groups={groups}
          placeholder="Jump to a session, lane or project"
          autoFocus
          onSelect={(id) => void select(id)}
        />
      </div>
    </ModalLayer>
  );
}
