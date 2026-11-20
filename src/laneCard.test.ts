import { describe, expect, it } from "vitest";
import { laneMainSession, laneResumeSessions } from "./laneCard";
import type { Session } from "./types";

const session = (key: string, over: Partial<Session> = {}): Session => ({
  key,
  id: Number(key.split(":")[1]),
  laneId: 1,
  harness: "pi",
  harnessRef: null,
  cwd: "/work",
  title: key,
  tags: [],
  createdAt: 1_000,
  updatedAt: 1_000,
  messageCount: 1,
  parentLocator: null,
  projectRoot: "/work",
  checkout: null,
  projectId: 1,
  pinned: false,
  archived: false,
  open: false,
  ...over,
});

describe("laneMainSession", () => {
  it("uses the open root session, otherwise the most recently active root", () => {
    const first = session("panoptes:1", { createdAt: 1_000, updatedAt: 1_000 });
    const later = session("panoptes:2", { createdAt: 2_000, updatedAt: 9_000 });
    const child = session("panoptes:3", { createdAt: 500, parentLocator: "/a.jsonl" });
    expect(laneMainSession([later, child, first])?.key).toBe("panoptes:2");
    expect(laneMainSession([later, { ...first, open: true }])?.key).toBe("panoptes:1");
  });

  it("breaks an activity tie by id and falls back to subagents when alone", () => {
    expect(laneMainSession([session("panoptes:5"), session("panoptes:4")])?.key).toBe("panoptes:5");
    const child = session("panoptes:7", { parentLocator: "/a.jsonl", createdAt: 3 });
    expect(
      laneMainSession([session("panoptes:8", { parentLocator: "/b", createdAt: 9 }), child])?.key,
    ).toBe("panoptes:8");
    expect(laneMainSession([])).toBeUndefined();
  });

  it("offers only earlier lane conversations, newest first", () => {
    const current = session("panoptes:1", { open: true, createdAt: 1 });
    const older = session("panoptes:2", { createdAt: 2 });
    const newest = session("panoptes:3", { createdAt: 3 });
    const archived = session("panoptes:4", { createdAt: 4, archived: true });
    const child = session("panoptes:5", { createdAt: 5, parentLocator: "/a" });
    expect(laneResumeSessions([older, child, current, newest, archived])).toEqual([newest, older]);
  });
});
