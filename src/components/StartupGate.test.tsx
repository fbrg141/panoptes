// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { StartupStatus } from "../store";
import { StartupGate } from "./StartupGate";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function render(status: StartupStatus, error: string | null = null, retry = vi.fn()) {
  await act(async () => {
    root.render(
      <StartupGate status={status} error={error} onRetry={retry}>
        <div>Workspace ready</div>
      </StartupGate>,
    );
  });
  return retry;
}

it("delays the sidebar and main-pane loaders by 150 ms", async () => {
  await render("loading");
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);

  await act(async () => vi.advanceTimersByTime(149));
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);

  await act(async () => vi.advanceTimersByTime(1));
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(2);
  expect(container.textContent).toContain("Loading projects and sessions…");
});

it("shows the workspace directly when startup succeeds", async () => {
  await render("ready");
  expect(container.textContent).toContain("Workspace ready");
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);
  expect(container.textContent).not.toContain("Loading projects and sessions…");
});

it("shows the startup error and retries without staying on the loader", async () => {
  const retry = await render("error", "Listing failed");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Listing failed");
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);

  await act(async () => {
    const button = [...container.querySelectorAll("button")].find(
      (item) => item.textContent === "Try again",
    );
    button?.click();
  });
  expect(retry).toHaveBeenCalledTimes(1);

  await render("loading", null, retry);
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);
});
