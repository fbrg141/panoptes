import type { CommandItem } from "@panoptes/design-system";
import type { Lane, Project, Session } from "./types";

/** What picking a palette row does; parsed back from the row's id. */
export type PaletteTarget =
  | { kind: "session"; session: Session }
  | { kind: "lane"; lane: Lane }
  | { kind: "project"; project: Project }
  | { kind: "newSession" }
  | { kind: "newProject" }
  | { kind: "shortcuts" };

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

/** One line under a session's title: its lane, its project, or its folder. */
function sessionHint(session: Session, projects: Project[], lanes: Lane[]): string {
  const parts: string[] = [];
  const lane = lanes.find((item) => item.id === session.laneId);
  if (lane) {
    parts.push(projects.find((item) => item.id === lane.projectId)?.name ?? "Project", lane.task);
  } else {
    parts.push(
      projects.find((item) => item.id === session.projectId)?.name ?? basename(session.cwd),
    );
  }
  if (session.archived) parts.unshift("Archived");
  return parts.join(" · ");
}

/**
 * Sessions in the order a switcher wants them: live ones before archived,
 * most recently updated first, and the one already on screen last, so Enter
 * on an empty query jumps to the session you were in before.
 */
export function recentSessions(sessions: Session[], activeKey: string | null = null): Session[] {
  const rank = (session: Session) => (session.key === activeKey ? 2 : session.archived ? 1 : 0);
  return [...sessions].sort((a, b) => rank(a) - rank(b) || b.updatedAt - a.updatedAt);
}

/**
 * The search palette's rows, grouped as the palette shows them. Sessions lead,
 * because jumping between them is what the palette is for; the actions follow
 * the things you can jump to. Ids carry the kind and the key or id of the
 * thing they stand for, so a pick can be traced back without closing over the
 * data.
 */
export function paletteGroups(
  sessions: Session[],
  projects: Project[],
  lanes: Lane[],
  activeKey: string | null = null,
): { label: string; items: CommandItem[] }[] {
  const projectName = (id: number) =>
    projects.find((project) => project.id === id)?.name ?? "Project";
  return [
    {
      label: "Sessions",
      items: recentSessions(sessions, activeKey).map((session) => {
        const hint = sessionHint(session, projects, lanes);
        return {
          id: `session:${session.key}`,
          label: session.title,
          hint: session.key === activeKey ? `Current · ${hint}` : hint,
          icon: "session",
        };
      }),
    },
    {
      label: "Lanes",
      items: lanes
        .filter((lane) => lane.state === "active")
        .map((lane) => {
          // A lane with no session yet starts one when picked; say so, since
          // the palette otherwise only jumps.
          const where = `${projectName(lane.projectId)} · ${lane.branch}`;
          const started = sessions.some((session) => session.laneId === lane.id);
          return {
            id: `lane:${lane.id}`,
            label: lane.task,
            hint: started ? where : `Start a session · ${where}`,
            icon: "git-branch",
          };
        }),
    },
    {
      label: "Projects",
      items: projects.map((project) => ({
        id: `project:${project.id}`,
        label: project.name,
        hint: `New lane · ${project.repo}`,
        icon: "folder",
      })),
    },
    {
      label: "Actions",
      items: [
        { id: "action:newSession", label: "New lane", icon: "plus", kbd: ["⌘", "T"] },
        { id: "action:newProject", label: "Add a project", icon: "folder", kbd: ["⌘", "N"] },
        {
          id: "action:shortcuts",
          label: "Keyboard shortcuts",
          icon: "command",
          kbd: ["⌘", "/"],
        },
      ],
    },
  ];
}

/** The thing a palette row's id stands for, or null if it is gone. */
export function paletteTarget(
  id: string,
  sessions: Session[],
  projects: Project[],
  lanes: Lane[],
): PaletteTarget | null {
  const [kind, rest] = [id.slice(0, id.indexOf(":")), id.slice(id.indexOf(":") + 1)];
  switch (kind) {
    case "session": {
      const session = sessions.find((item) => item.key === rest);
      return session ? { kind: "session", session } : null;
    }
    case "lane": {
      const lane = lanes.find((item) => item.id === Number(rest));
      return lane ? { kind: "lane", lane } : null;
    }
    case "project": {
      const project = projects.find((item) => item.id === Number(rest));
      return project ? { kind: "project", project } : null;
    }
    case "action":
      if (rest === "newSession") return { kind: "newSession" };
      if (rest === "newProject") return { kind: "newProject" };
      if (rest === "shortcuts") return { kind: "shortcuts" };
      return null;
    default:
      return null;
  }
}
