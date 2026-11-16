import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { singlePane } from "../panes";
import { useStore } from "../store";
import type { Project, Session } from "../types";
import { useUi } from "../ui";
import { sessionPickerGroups } from "./SessionPicker";
import { SessionWorkspace } from "./SessionWorkspace";

const session = {
  key: "pi:one",
  title: "Existing session",
  updatedAt: 10,
  archived: false,
} as Session;
const project = { id: 1, name: "panoptes" } as Project;

describe("split pane session choice", () => {
  beforeEach(() => {
    const pane = singlePane();
    useUi.setState({ layout: pane, focusedPaneId: pane.id, overlay: null });
    useStore.setState({ sessions: [], projects: [], conversations: {}, activeKey: null });
  });

  it("puts New session before every existing session", () => {
    const groups = sessionPickerGroups([session], [project]);
    expect(groups.map((group) => group.label)).toEqual(["Actions", "Sessions"]);
    expect(groups[0].items[0]).toMatchObject({
      id: "new",
      label: "New session",
      hint: "Start a lane in panoptes",
    });
    expect(groups[1].items[0]).toMatchObject({ id: "session:pi:one" });
  });

  it("offers project setup when New session has no project yet", () => {
    expect(sessionPickerGroups([], [])[0].items[0].hint).toBe("Add a project first");
  });

  it("leaves a quiet click-to-choose empty pane without the old action buttons", () => {
    const html = renderToStaticMarkup(<SessionWorkspace />);
    expect(html).toContain("No session selected");
    expect(html).toContain("Click to choose a session for this pane.");
    expect(html).toContain('aria-label="Choose a session for this pane"');
    expect(html).not.toContain(">Select session</button>");
    expect(html).not.toContain(">New lane</button>");
  });
});
