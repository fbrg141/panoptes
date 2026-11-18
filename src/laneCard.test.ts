import { describe, expect, it } from "vitest";
import { laneMainSession } from "./laneCard";
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
  it("keeps the first root session however recently others ran", () => {
    const first = session("panoptes:1", { createdAt: 1_000, updatedAt: 1_000 });
    const later = session("panoptes:2", { createdAt: 2_000, updatedAt: 9_000 });
    const child = session("panoptes:3", { createdAt: 500, parentLocator: "/a.jsonl" });
    // Activity order, as the backend lists them.
    expect(laneMainSession([later, child, first])?.key).toBe("panoptes:1");
  });

  it("breaks a creation tie by id and falls back to the oldest session", () => {
    expect(laneMainSession([session("panoptes:5"), session("panoptes:4")])?.key).toBe("panoptes:4");
    const child = session("panoptes:7", { parentLocator: "/a.jsonl", createdAt: 3 });
    expect(
      laneMainSession([session("panoptes:8", { parentLocator: "/b", createdAt: 9 }), child])?.key,
    ).toBe("panoptes:7");
    expect(laneMainSession([])).toBeUndefined();
  });
});
