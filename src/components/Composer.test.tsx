// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyConversation } from "../conversation";
import { useStore } from "../store";
import type { Session } from "../types";
import { useUi } from "../ui";
import { Composer } from "./Composer";

const laneSession: Session = {
  key: "pi:one",
  id: 1,
  laneId: 1,
  harness: "pi",
  harnessRef: null,
  cwd: "/work",
  title: "One",
  tags: [],
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/work",
  checkout: null,
  projectId: 1,
  pinned: false,
  archived: false,
  open: true,
};

let container: HTMLDivElement;
let root: Root;
const newLaneSession = vi.fn(async () => true);
const send = vi.fn(async () => true);

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  useStore.setState({
    conversations: { "pi:one": emptyConversation() },
    sessions: [laneSession],
    newLaneSession,
    send,
    settings: {},
    models: {},
    sending: {},
    drafts: {},
    activeKey: "pi:one",
  });
  useUi.setState({ overlay: null });
  newLaneSession.mockClear();
  send.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function submitCommand(command: string, busy = false) {
  act(() => root.render(<Composer sessionKey="pi:one" busy={busy} sending={false} />));
  const field = container.querySelector<HTMLTextAreaElement>("textarea");
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  if (!field || !setValue) throw new Error("The composer did not render");
  await act(async () => {
    setValue.call(field, command);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    container.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true }));
  });
}

it("catches /new instead of sending it to a busy agent", async () => {
  await submitCommand("/new", true);
  expect(newLaneSession).toHaveBeenCalledWith(1);
  expect(send).not.toHaveBeenCalled();
});

it("catches /resume and opens only the lane history picker", async () => {
  await submitCommand("/resume");
  expect(useUi.getState().overlay).toEqual({ type: "resumeSession", laneId: 1 });
  expect(send).not.toHaveBeenCalled();
});

it("restores an unsent draft when splitting remounts the composer", () => {
  const composer = <Composer sessionKey="pi:one" busy={false} sending={false} />;
  act(() => root.render(composer));
  const field = container.querySelector<HTMLTextAreaElement>("textarea");
  if (!field) throw new Error("The composer did not render");
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  if (!setValue) throw new Error("The textarea value setter is unavailable");

  act(() => {
    setValue.call(field, "Keep this unfinished prompt");
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(field.value).toBe("Keep this unfinished prompt");

  // A split wraps the old leaf in a new branch, unmounting this component.
  act(() => root.render(<div />));
  expect(useStore.getState().drafts["pi:one"]?.text).toBe("Keep this unfinished prompt");

  act(() => root.render(composer));
  const restored = container.querySelector<HTMLTextAreaElement>("textarea");
  expect(restored?.value).toBe("Keep this unfinished prompt");
});
