import { describe, expect, it } from "vitest";
import {
  closePane,
  dropSession,
  dropZoneAt,
  findPane,
  paneLeaves,
  placeSession,
  singlePane,
  splitPane,
} from "./panes";

const keys = (node: ReturnType<typeof singlePane> | Parameters<typeof paneLeaves>[0]) =>
  paneLeaves(node).map((pane) => pane.sessionKey);

describe("pane layout", () => {
  it("splits vertically and horizontally, keeping the old session first", () => {
    const root = singlePane("pi:a");
    const vertical = splitPane(root, root.id, "vertical");
    expect(vertical.layout).toMatchObject({ type: "split", direction: "vertical" });
    expect(keys(vertical.layout)).toEqual(["pi:a", null]);
    expect(vertical.newPaneId).toBe(paneLeaves(vertical.layout)[1].id);

    const horizontal = splitPane(vertical.layout, vertical.newPaneId ?? "", "horizontal");
    expect(horizontal.layout).toMatchObject({
      type: "split",
      direction: "vertical",
      second: { type: "split", direction: "horizontal" },
    });
    expect(keys(horizontal.layout)).toEqual(["pi:a", null, null]);
  });

  it("closes a leaf by collapsing its parent, and empties the last leaf", () => {
    const root = singlePane("pi:a");
    const { layout, newPaneId } = splitPane(root, root.id, "vertical");
    if (!newPaneId) throw new Error("Split did not create a pane");
    const withSession = placeSession(layout, newPaneId, "pi:b").layout;

    const closed = closePane(withSession, newPaneId);
    expect(closed.type).toBe("leaf");
    expect(keys(closed)).toEqual(["pi:a"]);
    expect(keys(closePane(closed, closed.id))).toEqual([null]);
  });

  it("places a session once and removes an empty destination when it is already visible", () => {
    const root = singlePane("pi:a");
    const split = splitPane(root, root.id, "horizontal");
    if (!split.newPaneId) throw new Error("Split did not create a pane");

    const placed = placeSession(split.layout, split.newPaneId, "pi:a");
    expect(keys(placed.layout)).toEqual(["pi:a"]);
    expect(placed.focusedPaneId).toBe(root.id);
  });

  it("fills a pane from the sidebar and splits on an edge drop", () => {
    const root = singlePane();
    const filled = dropSession(root, "pi:a", root.id, "center");
    expect(keys(filled.layout)).toEqual(["pi:a"]);

    const split = dropSession(filled.layout, "pi:b", root.id, "left");
    expect(split.layout).toMatchObject({ type: "split", direction: "vertical" });
    expect(keys(split.layout)).toEqual(["pi:b", "pi:a"]);
    expect(findPane(split.layout, split.focusedPaneId)?.sessionKey).toBe("pi:b");
  });

  it("swaps occupied panes on a centre drop", () => {
    const root = singlePane("pi:a");
    const split = splitPane(root, root.id, "vertical");
    if (!split.newPaneId) throw new Error("Split did not create a pane");
    const both = placeSession(split.layout, split.newPaneId, "pi:b").layout;

    const moved = dropSession(both, "pi:a", split.newPaneId, "center");
    expect(keys(moved.layout)).toEqual(["pi:b", "pi:a"]);
  });

  it("moves a visible session to an edge without duplicating it or losing the destination", () => {
    const root = singlePane("pi:a");
    const split = splitPane(root, root.id, "vertical");
    if (!split.newPaneId) throw new Error("Split did not create a pane");
    const both = placeSession(split.layout, split.newPaneId, "pi:b").layout;

    const moved = dropSession(both, "pi:a", split.newPaneId, "right");
    expect(keys(moved.layout)).toEqual(["pi:b", "pi:a"]);
    expect(paneLeaves(moved.layout)).toHaveLength(2);
  });

  it("moves a visible session into an empty pane without leaving an empty source", () => {
    const root = singlePane("pi:a");
    const split = splitPane(root, root.id, "vertical");
    if (!split.newPaneId) throw new Error("Split did not create a pane");

    const moved = dropSession(split.layout, "pi:a", split.newPaneId, "center");
    expect(keys(moved.layout)).toEqual(["pi:a"]);
    expect(moved.layout.type).toBe("leaf");
  });

  it("keeps a valid focused pane when a picker's destination has disappeared", () => {
    const root = singlePane("pi:a");
    expect(placeSession(root, "pane:gone", "pi:b")).toEqual({
      layout: root,
      focusedPaneId: root.id,
    });
  });

  it("uses the outer quarters as split zones", () => {
    const bounds = { left: 100, top: 50, width: 400, height: 200 };
    expect(dropZoneAt(bounds, 110, 100)).toBe("left");
    expect(dropZoneAt(bounds, 490, 100)).toBe("right");
    expect(dropZoneAt(bounds, 300, 60)).toBe("top");
    expect(dropZoneAt(bounds, 300, 240)).toBe("bottom");
    expect(dropZoneAt(bounds, 300, 150)).toBe("center");
  });
});
