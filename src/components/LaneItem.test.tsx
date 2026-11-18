// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { useStore } from "../store";
import type { Lane, LaneNavigation, Session } from "../types";
import { LaneItem } from "./Sidebar";

const now = Date.now();
const lane: Lane = {
  id: 2,
  projectId: 1,
  name: "parser",
  task: "Build the parser",
  branch: "panoptes/parser",
  state: "active",
  worktrees: [
    {
      projectId: 1,
      path: "/work/parser",
      mainCheckout: false,
      checkpointRef: null,
    },
  ],
};

const session = (over: Partial<Session> & Pick<Session, "key" | "title">): Session => ({
  id: 1,
  laneId: lane.id,
  harness: "pi",
  harnessRef: null,
  cwd: lane.worktrees[0].path,
  tags: [],
  createdAt: now - 60_000,
  updatedAt: now - 5 * 60_000,
  messageCount: 4,
  parentLocator: null,
  projectRoot: "/work/parser",
  checkout: "/work/repo",
  projectId: 1,
  pinned: false,
  archived: false,
  open: false,
  ...over,
});

const main = session({ key: "panoptes:1", title: "Main session title" });
const sub = session({
  key: "panoptes:2",
  id: 2,
  title: "Check the error recovery",
  parentLocator: "/sessions/main.jsonl",
  updatedAt: now - 10 * 60_000,
});

const navigation = (sessions: Session[]): LaneNavigation => ({
  laneId: lane.id,
  sessions,
  archivedSessions: [],
  work: { changedFiles: 2, commitsAhead: 1 },
  busyWriter: null,
});

let container: HTMLDivElement;
let root: Root;
let open: Mock<(session: Session) => Promise<void>>;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  open = vi.fn(async (_session: Session) => {});
  useStore.setState({
    activeKey: null,
    statuses: { [main.key]: "running", [sub.key]: "done" },
    open,
    close: vi.fn(async () => {}),
    setArchived: vi.fn(async () => {}),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(sessions: Session[], laneOverride: Partial<Lane> = {}) {
  act(() => {
    root.render(
      <nav aria-label="Lanes">
        <ul>
          <LaneItem
            lane={{ ...lane, ...laneOverride }}
            navigation={navigation(sessions)}
            onWork={vi.fn()}
            onAction={vi.fn()}
            onDelete={vi.fn()}
          />
        </ul>
      </nav>,
    );
  });
}

function rows(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll("[data-session-hit]"));
}

function disclosure(): HTMLButtonElement | null {
  return container.querySelector(
    '[aria-label="Expand lane parser"], [aria-label="Collapse lane parser"]',
  );
}

function click(element: HTMLElement) {
  act(() => element.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

describe("lane main session card", () => {
  it("opens a single-session lane in one click and has no disclosure", () => {
    render([main]);

    expect(disclosure()).toBeNull();
    expect(rows()).toHaveLength(1);
    const cardRows = rows();
    expect(cardRows[0].textContent).toContain("Build the parser");
    expect(cardRows[0].textContent).toContain("Running");
    expect(cardRows[0].textContent).toContain("5m ago");
    expect(cardRows[0].textContent).not.toContain("Main session title");

    click(cardRows[0]);
    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(main);
  });

  it("marks main and isolated checkouts on a session-backed lane", () => {
    const mainWorktree = [{ ...lane.worktrees[0], mainCheckout: true }];
    render([main], { worktrees: mainWorktree });

    expect(container.querySelector("[data-checkout='main']")?.className).toContain("text-waiting");
    expect(container.textContent).toContain("Main checkout");

    render([main]);
    expect(container.querySelector("[data-checkout='worktree']")?.className).toContain(
      "text-ink-muted",
    );
    expect(container.textContent).toContain("panoptes/parser");
  });

  it("marks the checkout kind on an empty lane too", () => {
    render([], { worktrees: [{ ...lane.worktrees[0], mainCheckout: true }] });
    expect(container.querySelector("[data-checkout='main']")).not.toBeNull();

    render([]);
    expect(container.querySelector("[data-checkout='worktree']")).not.toBeNull();
  });

  it("discloses only subsessions for a multi-session lane, even when one is more recent", () => {
    // Navigation is activity-sorted, so the root session is not necessarily first.
    render([sub, main]);

    expect(rows()).toHaveLength(1);
    expect(disclosure()?.getAttribute("aria-label")).toBe("Expand lane parser");
    click(disclosure() as HTMLButtonElement);
    expect(open).not.toHaveBeenCalled();

    expect(disclosure()?.getAttribute("aria-label")).toBe("Collapse lane parser");
    expect(rows()).toHaveLength(2);
    const expandedRows = rows();
    expect(expandedRows[0].textContent).toContain("Build the parser");
    expect(expandedRows[0].textContent).not.toContain("Main session title");
    expect(expandedRows[1].textContent).toContain("Check the error recovery");

    expandedRows[0].focus();
    act(() => {
      expandedRows[0].dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
      );
    });
    expect(document.activeElement).toBe(expandedRows[1]);
  });

  it("uses the main session's selection state on the lane card", () => {
    useStore.setState({ activeKey: main.key });
    render([main, sub]);

    const card = container.querySelector(".session-row");
    expect(card?.getAttribute("data-selected")).toBe("true");
    expect(rows()[0].getAttribute("aria-current")).toBe("page");

    act(() => useStore.setState({ activeKey: sub.key }));
    expect(card?.hasAttribute("data-selected")).toBe(false);
    expect(rows()[0].hasAttribute("aria-current")).toBe(false);
  });
});
