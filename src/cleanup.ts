import type { WorktreeUsage } from "./types";

const KIB = 1_024;
const DAY_MS = 24 * 60 * 60 * 1_000;

/** Binary units, matching the allocated blocks reported by `du -sk`. */
export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= KIB && unit < units.length - 1) {
    value /= KIB;
    unit += 1;
  }
  const digits = unit > 0 && value < 10 && !Number.isInteger(value) ? 1 : 0;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function usageLabel(usage: WorktreeUsage): string {
  return `${usage.worktrees} ${usage.worktrees === 1 ? "worktree" : "worktrees"} · ${formatBytes(usage.bytes)}`;
}

export function idleLabel(lastActivity: number, now = Date.now()): string {
  const days = Math.max(0, Math.floor((now - lastActivity) / DAY_MS));
  return `${days} ${days === 1 ? "day" : "days"} idle`;
}
