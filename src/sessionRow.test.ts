import { describe, expect, it } from "vitest";
import {
  isLive,
  lockLabel,
  lockQueueText,
  primaryAction,
  sessionDetail,
  sessionRowLabel,
  statusWord,
} from "./sessionRow";

describe("session row", () => {
  it("gives weight only to sessions at work or waiting on you", () => {
    expect(["running", "waiting", "failed"].every((s) => isLive(s as never))).toBe(true);
    expect(isLive("done")).toBe(false);
    expect(isLive("idle")).toBe(false);
  });

  it("uses the guide's status words and none for idle", () => {
    expect(statusWord("running")).toBe("Running");
    expect(statusWord("waiting")).toBe("Waiting");
    expect(statusWord("failed")).toBe("Failed");
    expect(statusWord("done")).toBe("Done");
    expect(statusWord("idle")).toBeNull();
  });

  it("names the lock only when it matters", () => {
    expect(lockLabel(undefined)).toBeNull();
    expect(lockLabel({ holder: false, queued: false })).toBeNull();
    expect(lockLabel({ holder: true, queued: false })).toBe("Holds the lane's write lock");
    expect(lockLabel({ holder: false, queued: true })).toMatch(/read-only/);
  });

  it("shows which sessions are waiting for the write lock", () => {
    expect(lockQueueText(undefined)).toBeNull();
    expect(lockQueueText({ holder: false, queued: false })).toBeNull();
    expect(lockQueueText({ holder: true, queued: false })).toBeNull();
    expect(lockQueueText({ holder: false, queued: true })).toBe("Waiting");
  });

  it("does not repeat the lane's project when nested", () => {
    expect(sessionDetail({ archived: false, messageCount: 12 }, undefined)).toBe("12 messages");
    expect(sessionDetail({ archived: false, messageCount: 1 }, undefined)).toBe("1 message");
    expect(sessionDetail({ archived: true, messageCount: 3 }, "panoptes / Lane")).toBe(
      "Archived · panoptes / Lane",
    );
  });

  it("puts the title first and the status in the accessible name", () => {
    expect(
      sessionRowLabel({
        title: "Fix scroll",
        status: "waiting",
        lock: { holder: true, queued: false },
        detail: "panoptes",
        time: "2m ago",
      }),
    ).toBe("Fix scroll, Waiting, Holds the lane's write lock, panoptes, updated 2m ago");
    expect(
      sessionRowLabel({
        title: "Idle",
        status: "idle",
        lock: undefined,
        detail: "x",
        time: "1d ago",
      }),
    ).toBe("Idle, x, updated 1d ago");
  });

  it("offers Stop on the row only while an open session runs", () => {
    expect(primaryAction({ open: true }, "running")).toMatchObject({ id: "stop" });
    expect(primaryAction({ open: false }, "running")).toBeNull();
    expect(primaryAction({ open: true }, "waiting")).toBeNull();
  });
});
