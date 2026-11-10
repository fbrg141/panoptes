// What the new session form offers and where it starts, kept out of the
// component so it can be tested without a window.
import type { PickerGroup } from "./picker";
import type { Lane, ModelInfo, Project, Session } from "./types";

/** The lane picker's last row, which opens the new lane form instead. */
export const NEW_LANE = "action:new-lane";
/** The model picker's first row: leave the choice to the harness. */
export const DEFAULT_CHOICE = "";

const home = (path: string) => path.replace(/^\/Users\/[^/]+/, "~");

/**
 * Where a new session starts: the lane of the session on screen, else the
 * first lane of the first project that has one, else the first project with
 * no lane.
 */
export function defaultTarget(
  projects: Project[],
  lanes: Lane[],
  sessions: Session[],
  activeKey: string | null,
): { projectId: number | null; laneId: number | null } {
  const active = lanes.filter(
    (lane) => lane.state === "active" && projects.some((project) => project.id === lane.projectId),
  );
  const current = sessions.find((session) => session.key === activeKey);
  const lane = active.find((item) => item.id === current?.laneId) ?? active[0];
  if (lane) return { projectId: lane.projectId, laneId: lane.id };
  return { projectId: projects[0]?.id ?? null, laneId: null };
}

/** The first active lane of `projectId`, or null. */
export function firstLane(lanes: Lane[], projectId: number | null): number | null {
  return lanes.find((lane) => lane.state === "active" && lane.projectId === projectId)?.id ?? null;
}

export function projectGroups(projects: Project[], projectId: number | null): PickerGroup[] {
  return [
    {
      items: projects.map((project) => ({
        id: String(project.id),
        label: project.name,
        detail: home(project.repo),
        current: project.id === projectId,
      })),
    },
  ];
}

/** The project's active lanes, then a way to make a new one. */
export function laneGroups(
  lanes: Lane[],
  projectId: number | null,
  laneId: number | null,
): PickerGroup[] {
  const here = lanes.filter((lane) => lane.state === "active" && lane.projectId === projectId);
  return [
    {
      items: here.map((lane) => ({
        id: String(lane.id),
        label: lane.task,
        detail: lane.branch,
        current: lane.id === laneId,
      })),
    },
    {
      label: here.length > 0 ? undefined : "No active lanes",
      items: [{ id: NEW_LANE, label: "New lane…" }],
    },
  ];
}

/** Models by provider, after a row that keeps the harness's default. */
export function modelGroups(models: ModelInfo[], modelId: string): PickerGroup[] {
  const groups: PickerGroup[] = [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "Harness default", current: modelId === DEFAULT_CHOICE },
      ],
    },
  ];
  for (const model of models) {
    let group = groups.find((item) => item.label === model.provider);
    if (!group) {
      group = { label: model.provider, items: [] };
      groups.push(group);
    }
    group.items.push({
      id: model.id,
      label: model.name,
      detail: shortModelId(model),
      keywords: [model.provider, model.id],
      current: model.id === modelId,
    });
  }
  return groups;
}

/** A model's id without its provider prefix, which its group already names. */
export function shortModelId(model: Pick<ModelInfo, "id" | "provider">): string {
  return model.id.startsWith(`${model.provider}/`)
    ? model.id.slice(model.provider.length + 1)
    : model.id;
}

export function thinkingGroups(levels: string[], level: string): PickerGroup[] {
  return [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "Model default", current: level === DEFAULT_CHOICE },
        ...levels.map((item) => ({ id: item, label: item, current: item === level })),
      ],
    },
  ];
}
