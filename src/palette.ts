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
 * The search palette's rows, grouped as the palette shows them. Ids carry the
 * kind and the key or id of the thing they stand for, so a pick can be traced
 * back without closing over the data.
 */
export function paletteGroups(
  sessions: Session[],
  projects: Project[],
  lanes: Lane[],
): { label: string; items: CommandItem[] }[] {
  return [
    {
      label: "Actions",
      items: [
        { id: "action:newSession", label: "New session", icon: "plus", kbd: ["⌘", "T"] },
        { id: "action:newProject", label: "Add a project", icon: "folder", kbd: ["⌘", "N"] },
        {
          id: "action:shortcuts",
          label: "Keyboard shortcuts",
          icon: "command",
          kbd: ["⌘", "/"],
        },
      ],
    },
    {
      label: "Sessions",
      items: sessions.map((session) => ({
        id: `session:${session.key}`,
        label: session.title,
        hint: sessionHint(session, projects, lanes),
        icon: "session",
      })),
    },
    {
      label: "Lanes",
      items: lanes
        .filter((lane) => lane.state === "active")
        .map((lane) => ({
          id: `lane:${lane.id}`,
          label: lane.task,
          hint: `${projects.find((project) => project.id === lane.projectId)?.name ?? "Project"} · ${lane.branch}`,
          icon: "git-branch",
        })),
    },
    {
      label: "Projects",
      items: projects.map((project) => ({
        id: `project:${project.id}`,
        label: project.name,
        hint: project.repo,
        icon: "folder",
      })),
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
