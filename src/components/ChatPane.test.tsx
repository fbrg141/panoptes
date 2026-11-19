// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyConversation } from "../conversation";
import { useStore } from "../store";
import type { Session } from "../types";
import { ChatPane } from "./ChatPane";

vi.mock("./Composer", () => ({ Composer: () => null }));

const session: Session = {
  key: "pi:s1",
  id: null,
  laneId: null,
  harness: "pi",
  harnessRef: { harness: "pi", id: "s1", locator: "/s1.jsonl", cwd: "/code" },
  cwd: "/code",
  title: "One",
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
};

let container: HTMLDivElement;
let root: Root;
const open = vi.fn(async () => {});
const dismissPermissionWarning = vi.fn(async () => {});

beforeEach(async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
  } as unknown as typeof ResizeObserver;
  open.mockClear();
  dismissPermissionWarning.mockClear();
  useStore.setState({
    sessions: [session],
    conversations: {},
    openErrors: {},
    unguardedTools: {},
    preferences: {
      profiles: [],
      favoriteModels: [],
      hiddenModels: [],
      cleanupIdleDays: 4,
      permissionWarningDismissed: false,
    },
    dismissPermissionWarning,
    statuses: {},
    sending: {},
    activeKey: session.key,
    open,
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<ChatPane sessionKey={session.key} />));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it("shows a failed open instead of loading forever and can retry it", async () => {
  expect(container.textContent).toContain("Starting the agent…");

  await act(async () => {
    useStore.setState({ openErrors: { [session.key]: "Could not start" } });
  });
  expect(container.textContent).not.toContain("Starting the agent…");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Could not start");

  await act(async () => {
    const retry = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "Try again",
    );
    retry?.click();
  });
  expect(open).toHaveBeenCalledWith(session);
});

it("warns only for unguarded tools and remembers dismissal", async () => {
  await act(async () => {
    useStore.setState({ conversations: { [session.key]: emptyConversation() } });
  });
  expect(container.textContent).not.toContain("No permission extension installed");

  await act(async () => {
    useStore.setState({ unguardedTools: { [session.key]: true } });
  });
  expect(container.textContent).toContain(
    "No permission extension installed: bash, edit and write run without asking.",
  );
  const install = container.querySelector<HTMLAnchorElement>("a[href*='pi-permission-system']");
  expect(install?.textContent).toBe("pi install npm:pi-permission-system");

  await act(async () => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Dismiss permission warning"]')
      ?.click();
  });
  expect(dismissPermissionWarning).toHaveBeenCalledTimes(1);

  await act(async () => {
    useStore.setState((state) => ({
      preferences: { ...state.preferences, permissionWarningDismissed: true },
    }));
  });
  expect(container.textContent).not.toContain("No permission extension installed");
});
