// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { useStore } from "../store";
import type { Lane, LaneCleanup, Project } from "../types";
import { CleanupDialog } from "./CleanupDialog";

const lane: Lane = {
  id: 2,
  projectId: 1,
  name: "old-docs",
  task: "Update old documentation",
  branch: "panoptes/old-docs",
  state: "active",
  worktrees: [
    {
      projectId: 1,
      path: "/lanes/old-docs/web",
      mainCheckout: false,
      checkpointRef: null,
    },
  ],
};

const project: Project = {
  id: 1,
  repo: "/code/web",
  name: "web",
  baseBranch: "main",
  copyFiles: [],
  setup: null,
};

const cleanup: LaneCleanup = {
  project: { worktrees: 1, bytes: 5 * 1024 ** 3 },
  total: { worktrees: 2, bytes: 8 * 1024 ** 3 },
  idleDays: 4,
  candidates: [
    {
      lane,
      lastActivity: Date.now() - 9 * 24 * 60 * 60 * 1_000,
      bytes: 5 * 1024 ** 3,
      preservation: "checkpoint",
    },
  ],
  errors: [],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

function button(text: string): HTMLButtonElement {
  const found = [...container.querySelectorAll("button")].find((item) =>
    item.textContent?.includes(text),
  );
  if (!found) throw new Error(`No button containing ${text}`);
  return found as HTMLButtonElement;
}

describe("lane cleanup dialog", () => {
  it("requires selection and confirmation before checkpoint-safe archiving", async () => {
    vi.spyOn(api, "laneCleanup")
      .mockResolvedValueOnce(cleanup)
      .mockResolvedValueOnce({ ...cleanup, candidates: [] });
    const archiveLanes = vi.fn(async () => ({ archived: 1, failed: [] }));
    useStore.setState({ projects: [project], lanes: [lane], archiveLanes });

    await act(async () => {
      root.render(<CleanupDialog projectId={1} />);
    });
    await vi.waitFor(() => expect(container.textContent).toContain("Update old documentation"));

    expect(container.textContent).toContain("1 worktree · 5 GB");
    expect(container.textContent).toContain("2 worktrees · 8 GB");
    expect(container.textContent).toContain("Uncommitted changes will be checkpointed");
    expect(button("Archive selected").disabled).toBe(true);

    const checkbox = container.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox?.checked).toBe(false);
    await act(async () => checkbox?.click());
    expect(button("Archive selected (1)").disabled).toBe(false);
    expect(archiveLanes).not.toHaveBeenCalled();

    await act(async () => button("Archive selected (1)").click());
    expect(container.textContent).toContain("Their worktrees will be removed");
    expect(archiveLanes).not.toHaveBeenCalled();

    await act(async () => button("Confirm archive (1)").click());
    await vi.waitFor(() => expect(container.textContent).toContain("Archived 1 lane."));
    expect(archiveLanes).toHaveBeenCalledOnce();
    expect(archiveLanes).toHaveBeenCalledWith([2]);
    expect(container.textContent).toContain("No safe idle lanes to archive.");
  });
});
