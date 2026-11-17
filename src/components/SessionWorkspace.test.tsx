// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyEvent, emptyConversation } from "../conversation";
import { singlePane, splitPane } from "../panes";
import { useStore } from "../store";
import type { HarnessEvent, Message } from "../types";
import { useUi } from "../ui";
import { SessionWorkspace } from "./SessionWorkspace";

// Count renders of the leaves each pane draws. The real Markdown is memoized
// on its text, which would hide a turn re-rendering; this stand-in is not.
const renders = vi.hoisted(() => ({
  markdown: new Map<string, number>(),
  composer: new Map<string, number>(),
}));
const bump = (map: Map<string, number>, key: string) => map.set(key, (map.get(key) ?? 0) + 1);

vi.mock("./Markdown", () => ({
  Markdown: ({ text }: { text: string }) => {
    bump(renders.markdown, text);
    return <div>{text}</div>;
  },
}));
vi.mock("./Composer", () => ({
  Composer: ({ sessionKey }: { sessionKey: string }) => {
    bump(renders.composer, sessionKey);
    return null;
  },
}));

const message = (id: string, role: Message["role"], text: string): Message => ({
  id,
  role,
  blocks: [{ type: "text", text }],
  timestamp: null,
  error: null,
  stopped: false,
  usage: null,
});

const streaming = "pi:a";
const other = "pi:b";

let container: HTMLDivElement;
let root: Root;

const stream = (event: HarnessEvent) =>
  act(() => {
    // The same update the store makes for a session event.
    useStore.setState((s) => ({
      conversations: {
        ...s.conversations,
        [streaming]: applyEvent(s.conversations[streaming], event),
      },
    }));
  });

beforeEach(async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
  } as unknown as typeof ResizeObserver;
  renders.markdown.clear();
  renders.composer.clear();

  const first = singlePane(streaming);
  const split = splitPane(first, first.id, "vertical");
  const layout =
    split.layout.type === "split"
      ? { ...split.layout, second: { ...split.layout.second, sessionKey: other } }
      : split.layout;
  useUi.setState({ layout, focusedPaneId: first.id, overlay: null });
  useStore.setState({
    activeKey: null,
    sessions: [],
    statuses: {},
    sending: {},
    conversations: {
      [streaming]: {
        ...emptyConversation([
          message("a-u1", "user", "First question"),
          message("a-r1", "assistant", "Finished answer in A"),
          message("a-u2", "user", "Second question"),
          message("a-r2", "assistant", "Hel"),
        ]),
        busy: true,
      },
      [other]: emptyConversation([
        message("b-u1", "user", "Question in B"),
        message("b-r1", "assistant", "Answer in B"),
      ]),
    },
  });

  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<SessionWorkspace />));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it("re-renders only the streaming turn of the streaming pane", async () => {
  expect(renders.composer.get(other)).toBe(1);
  expect(renders.markdown.get("Answer in B")).toBe(1);
  expect(renders.markdown.get("Finished answer in A")).toBe(1);

  for (const delta of ["l", "o", " there"]) {
    await stream({ type: "blockDelta", messageId: "a-r2", index: 0, kind: "text", delta });
  }

  expect(renders.markdown.get("Hello there")).toBe(1);
  // The other pane and the finished turn are untouched by the tokens.
  expect(renders.composer.get(other)).toBe(1);
  expect(renders.markdown.get("Answer in B")).toBe(1);
  expect(renders.markdown.get("Finished answer in A")).toBe(1);
  // The streaming pane itself keeps updating.
  expect(renders.composer.get(streaming)).toBe(4);
});

it("still closes the pane of a session that is closed", async () => {
  await act(async () => {
    useStore.setState((s) => {
      const { [other]: _closed, ...conversations } = s.conversations;
      return { conversations };
    });
  });
  expect(container.querySelectorAll("[data-pane]")).toHaveLength(1);
});
