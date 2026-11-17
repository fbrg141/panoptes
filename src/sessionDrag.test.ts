// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { paneLeaves, singlePane } from "./panes";
import { beginSessionDrag, sessionDragSnapshot } from "./sessionDrag";
import { useStore } from "./store";
import type { Session } from "./types";
import { useUi } from "./ui";

let open: Mock<(session: Session) => Promise<void>>;

let pane: HTMLElement;
let originalElementFromPoint: typeof document.elementFromPoint;

function point(x: number, y: number): PointerEvent {
  return new MouseEvent("pointermove", {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
  }) as PointerEvent;
}

function move(x: number, y: number) {
  window.dispatchEvent(point(x, y));
}

function release(x: number, y: number) {
  window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, clientX: x, clientY: y }));
}

beforeEach(() => {
  const layout = singlePane("pi:a");
  useUi.setState({ layout, focusedPaneId: layout.id });
  useStore.setState({ activeKey: null });
  open = vi.fn(async (_session: Session) => {});
  useStore.setState({ open, sessions: [] });
  pane = document.createElement("section");
  pane.dataset.pane = layout.id;
  pane.getBoundingClientRect = () => ({ left: 0, top: 0, width: 400, height: 300 }) as DOMRect;
  document.body.append(pane);
  originalElementFromPoint = document.elementFromPoint;
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn(() => pane),
  });
});

afterEach(() => {
  window.dispatchEvent(new MouseEvent("pointercancel", { bubbles: true }));
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: originalElementFromPoint,
  });
  pane.remove();
  document.body.style.userSelect = "";
});

describe("pointer session dragging", () => {
  it("drops a sidebar session onto a vertical edge without HTML5 drag events", () => {
    const session = { key: "pi:b" } as Session;
    useStore.setState({ sessions: [session] });
    beginSessionDrag({ button: 0, clientX: 0, clientY: 0 }, "pi:b");
    move(390, 150);

    expect(sessionDragSnapshot()).toEqual({
      sessionKey: "pi:b",
      paneId: pane.dataset.pane,
      zone: "right",
    });
    release(390, 150);

    const state = useUi.getState();
    expect(state.layout.type).toBe("split");
    expect(state.layout.type === "split" && state.layout.direction).toBe("vertical");
    expect(paneLeaves(state.layout).map((leaf) => leaf.sessionKey)).toEqual(["pi:a", "pi:b"]);
    expect(useStore.getState().activeKey).toBe("pi:b");
    expect(open).toHaveBeenCalledWith(session);
    expect(sessionDragSnapshot().sessionKey).toBeNull();
  });

  it("moves a visible session without opening another process", () => {
    const session = { key: "pi:b" } as Session;
    useStore.setState({ sessions: [session] });
    useUi.getState().dropSession("pi:b", pane.dataset.pane as string, "right");
    open.mockClear();

    beginSessionDrag({ button: 0, clientX: 0, clientY: 0 }, "pi:b");
    move(200, 150);
    release(200, 150);

    expect(
      paneLeaves(useUi.getState().layout)
        .map((leaf) => leaf.sessionKey)
        .sort(),
    ).toEqual(["pi:a", "pi:b"]);
    expect(open).not.toHaveBeenCalled();
  });

  it("uses the same horizontal drop zones and suppresses the compatibility click", () => {
    beginSessionDrag({ button: 0, clientX: 0, clientY: 0 }, "pi:b");
    move(200, 20);
    release(200, 20);

    const state = useUi.getState();
    expect(state.layout.type).toBe("split");
    expect(state.layout.type === "split" && state.layout.direction).toBe("horizontal");
    expect(paneLeaves(state.layout).map((leaf) => leaf.sessionKey)).toEqual(["pi:b", "pi:a"]);

    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    window.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
  });

  it("keeps an ordinary pointer click as a click", () => {
    const before = useUi.getState().layout;
    beginSessionDrag({ button: 0, clientX: 10, clientY: 10 }, "pi:b");
    move(11, 11);
    release(11, 11);

    expect(useUi.getState().layout).toBe(before);
    expect(useStore.getState().activeKey).toBeNull();
    expect(sessionDragSnapshot().sessionKey).toBeNull();
  });

  it("cancels without changing the pane layout", () => {
    const before = useUi.getState().layout;
    beginSessionDrag({ button: 0, clientX: 0, clientY: 0 }, "pi:b");
    move(390, 150);
    window.dispatchEvent(
      new MouseEvent("pointercancel", { bubbles: true, clientX: 390, clientY: 150 }),
    );

    expect(useUi.getState().layout).toBe(before);
    expect(useStore.getState().activeKey).toBeNull();
    expect(sessionDragSnapshot().sessionKey).toBeNull();
  });
});
