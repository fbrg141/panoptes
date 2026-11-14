import { beforeEach, describe, expect, it } from "vitest";
import { paneLeaves, singlePane } from "./panes";
import { useUi } from "./ui";

describe("ui overlays", () => {
  beforeEach(() => {
    useUi.getState().close();
    const pane = singlePane();
    useUi.setState({ layout: pane, focusedPaneId: pane.id });
  });

  it("opens one overlay at a time and closes it", () => {
    useUi.getState().open({ type: "search" });
    expect(useUi.getState().overlay).toEqual({ type: "search" });

    useUi.getState().open({ type: "shortcuts" });
    expect(useUi.getState().overlay).toEqual({ type: "shortcuts" });

    useUi.getState().close();
    expect(useUi.getState().overlay).toBeNull();
  });

  it("carries the project a lane form was opened for", () => {
    useUi.getState().open({ type: "newLane", projectId: 3 });
    expect(useUi.getState().overlay).toEqual({ type: "newLane", projectId: 3 });
  });

  it("carries a prompt typed before the lane form took over", () => {
    useUi.getState().open({ type: "newLane", projectId: 3, prompt: "Fix login" });
    expect(useUi.getState().overlay).toEqual({
      type: "newLane",
      projectId: 3,
      prompt: "Fix login",
    });
  });

  it("splits and focuses the new pane, then fills and closes it", () => {
    useUi.getState().showSession("pi:a");
    const paneId = useUi.getState().splitFocused("vertical");
    expect(paneId).not.toBeNull();
    expect(useUi.getState().focusedPaneId).toBe(paneId);
    expect(paneLeaves(useUi.getState().layout).map((pane) => pane.sessionKey)).toEqual([
      "pi:a",
      null,
    ]);

    useUi.getState().showSession("pi:b");
    expect(paneLeaves(useUi.getState().layout).map((pane) => pane.sessionKey)).toEqual([
      "pi:a",
      "pi:b",
    ]);
    useUi.getState().closePane(paneId ?? "");
    expect(paneLeaves(useUi.getState().layout).map((pane) => pane.sessionKey)).toEqual(["pi:a"]);
  });

  it("carries the destination pane through the session picker overlay", () => {
    useUi.getState().open({ type: "paneSession", paneId: "pane-3" });
    expect(useUi.getState().overlay).toEqual({ type: "paneSession", paneId: "pane-3" });
  });
});
