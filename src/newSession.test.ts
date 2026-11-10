import { describe, expect, it } from "vitest";
import {
  DEFAULT_CHOICE,
  defaultTarget,
  firstLane,
  laneGroups,
  modelGroups,
  NEW_LANE,
  projectGroups,
  shortModelId,
  thinkingGroups,
} from "./newSession";
import type { Lane, ModelInfo, Project, Session } from "./types";

const project = (id: number, name: string): Project => ({
  id,
  repo: `/Users/me/code/${name}`,
  name,
  baseBranch: "main",
  copyFiles: [],
  setup: null,
});

const lane = (id: number, projectId: number, state: Lane["state"] = "active"): Lane => ({
  id,
  projectId,
  name: `lane-${id}`,
  task: `Task ${id}`,
  branch: `panoptes/lane-${id}`,
  state,
  worktrees: [],
  writeLock: { holder: null, queue: [] },
});

const session = (key: string, laneId: number | null): Session => ({
  key,
  id: null,
  laneId,
  harness: "pi",
  harnessRef: null,
  cwd: "/code",
  title: key,
  tags: [],
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/code",
  checkout: null,
  projectId: null,
  pinned: false,
  archived: false,
  open: false,
});

const model = (id: string, name: string, provider: string): ModelInfo => ({
  id,
  name,
  provider,
  contextWindow: null,
  images: true,
});

const projects = [project(1, "web"), project(2, "api")];
const lanes = [lane(10, 1, "archived"), lane(11, 2), lane(12, 2), lane(13, 1)];

describe("defaultTarget", () => {
  it("starts in the lane of the session on screen", () => {
    expect(defaultTarget(projects, lanes, [session("s", 12)], "s")).toEqual({
      projectId: 2,
      laneId: 12,
    });
  });

  it("falls back to the first active lane, then to a project with none", () => {
    expect(defaultTarget(projects, lanes, [], null)).toEqual({ projectId: 2, laneId: 11 });
    expect(defaultTarget(projects, [lane(10, 1, "archived")], [], null)).toEqual({
      projectId: 1,
      laneId: null,
    });
    expect(defaultTarget([], [], [], null)).toEqual({ projectId: null, laneId: null });
  });
});

describe("the form's pickers", () => {
  it("finds a project's first active lane", () => {
    expect(firstLane(lanes, 1)).toBe(13);
    expect(firstLane(lanes, 9)).toBeNull();
  });

  it("lists projects with a home-relative path and ticks the chosen one", () => {
    const [group] = projectGroups(projects, 2);
    expect(group.items[0]).toMatchObject({ id: "1", detail: "~/code/web", current: false });
    expect(group.items[1].current).toBe(true);
  });

  it("lists a project's active lanes and always ends with a new lane row", () => {
    const groups = laneGroups(lanes, 1, 13);
    expect(groups[0].items.map((item) => item.id)).toEqual(["13"]);
    expect(groups[0].items[0]).toMatchObject({ detail: "panoptes/lane-13", current: true });
    expect(groups[1]).toEqual({ label: undefined, items: [{ id: NEW_LANE, label: "New lane…" }] });
    expect(laneGroups(lanes, 9, null)[1].label).toBe("No active lanes");
  });

  it("groups models by provider after the harness default", () => {
    const groups = modelGroups(
      [
        model("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5", "anthropic"),
        model("local/qwen", "Qwen", "local"),
        model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5", "anthropic"),
      ],
      "local/qwen",
    );
    expect(groups.map((group) => group.label)).toEqual([undefined, "anthropic", "local"]);
    expect(groups[0].items[0]).toMatchObject({ id: DEFAULT_CHOICE, current: false });
    expect(groups[1].items.map((item) => item.detail)).toEqual([
      "claude-sonnet-4-5",
      "claude-haiku-4-5",
    ]);
    expect(groups[2].items[0].current).toBe(true);
  });

  it("shortens a model id only when it carries its provider", () => {
    expect(shortModelId({ id: "openai/gpt-5", provider: "openai" })).toBe("gpt-5");
    expect(shortModelId({ id: "gpt-5", provider: "openai" })).toBe("gpt-5");
  });

  it("offers the model's default before its thinking levels", () => {
    const [group] = thinkingGroups(["low", "high"], "high");
    expect(group.items.map((item) => item.id)).toEqual([DEFAULT_CHOICE, "low", "high"]);
    expect(group.items[2].current).toBe(true);
  });
});
