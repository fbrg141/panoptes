import { beforeEach, describe, expect, it } from "vitest";
import { useUi } from "./ui";

describe("ui overlays", () => {
  beforeEach(() => {
    useUi.getState().close();
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
});
