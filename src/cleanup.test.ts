import { describe, expect, it } from "vitest";
import { formatBytes, idleLabel, usageLabel } from "./cleanup";

describe("lane cleanup labels", () => {
  it("formats allocated bytes without false precision", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1_024)).toBe("1 KB");
    expect(formatBytes(1_536)).toBe("1.5 KB");
    expect(formatBytes(27 * 1_024 ** 3)).toBe("27 GB");
    expect(formatBytes(2 * 1_024 ** 4)).toBe("2 TB");
  });

  it("labels worktree counts and whole idle days", () => {
    expect(usageLabel({ worktrees: 1, bytes: 1_024 })).toBe("1 worktree · 1 KB");
    expect(usageLabel({ worktrees: 2, bytes: 2_048 })).toBe("2 worktrees · 2 KB");
    const now = 10 * 24 * 60 * 60 * 1_000;
    expect(idleLabel(now - 4 * 24 * 60 * 60 * 1_000, now)).toBe("4 days idle");
    expect(idleLabel(now - 24 * 60 * 60 * 1_000, now)).toBe("1 day idle");
  });
});
