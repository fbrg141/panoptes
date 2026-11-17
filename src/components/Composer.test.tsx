// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { emptyConversation } from "../conversation";
import { useStore } from "../store";
import { Composer } from "./Composer";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  useStore.setState({
    conversations: { "pi:one": emptyConversation() },
    sessions: [],
    settings: {},
    models: {},
    sending: {},
    drafts: {},
    activeKey: "pi:one",
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
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
