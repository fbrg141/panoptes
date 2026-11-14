import { Button, Icon, LoopMark } from "@panoptes/design-system";
import { useEffect, useRef, useState } from "react";
import {
  type DropZone,
  dropZoneAt,
  findPane,
  type PaneNode,
  paneLeaves,
  SESSION_DRAG_TYPE,
  type SplitDirection,
  sessionPane,
} from "../panes";
import { useStore } from "../store";
import { useUi } from "../ui";
import { ChatPane } from "./ChatPane";

/** The tiled session area. Pane layout is view state; the store owns sessions. */
export function SessionWorkspace() {
  const layout = useUi((s) => s.layout);
  const focusedPaneId = useUi((s) => s.focusedPaneId);
  const activeKey = useStore((s) => s.activeKey);
  const conversations = useStore((s) => s.conversations);
  const previousConversations = useRef(conversations);

  // Creating a session still goes through the store's active key. Adopt that
  // result in the focused pane; ordinary row and palette selections also place
  // themselves explicitly so selecting the same key still works.
  useEffect(() => {
    if (activeKey) useUi.getState().showSession(activeKey);
  }, [activeKey]);

  // Closing or archiving a session forgets its conversation. Remove a pane
  // only after a conversation disappears, never while its first open is pending.
  useEffect(() => {
    const previous = previousConversations.current;
    previousConversations.current = conversations;
    for (const key of Object.keys(previous)) {
      if (conversations[key]) continue;
      const pane = sessionPane(useUi.getState().layout, key);
      if (pane) closeAndFocus(pane.id);
    }
  }, [conversations]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <PaneNodeView
        node={layout}
        focusedPaneId={focusedPaneId}
        onActivate={activatePane}
        onSplit={splitFocusedPane}
        onClose={closeAndFocus}
      />
    </div>
  );
}

function activatePane(paneId: string) {
  useUi.getState().focusPane(paneId);
  useStore.setState({
    activeKey: findPane(useUi.getState().layout, paneId)?.sessionKey ?? null,
  });
}

function splitFocusedPane(direction: SplitDirection) {
  const paneId = useUi.getState().splitFocused(direction);
  if (paneId) useStore.setState({ activeKey: null });
}

function closeAndFocus(paneId: string) {
  useUi.getState().closePane(paneId);
  const state = useUi.getState();
  useStore.setState({
    activeKey: findPane(state.layout, state.focusedPaneId)?.sessionKey ?? null,
  });
}

function PaneNodeView({
  node,
  focusedPaneId,
  onActivate,
  onSplit,
  onClose,
}: {
  node: PaneNode;
  focusedPaneId: string;
  onActivate: (paneId: string) => void;
  onSplit: (direction: SplitDirection) => void;
  onClose: (paneId: string) => void;
}) {
  if (node.type === "split") {
    return (
      <div
        className={`flex min-h-0 min-w-0 flex-1 ${
          node.direction === "vertical" ? "flex-row" : "flex-col"
        }`}
        data-split={node.direction}
      >
        <div className="flex min-h-0 min-w-0 flex-1">
          <PaneNodeView
            node={node.first}
            focusedPaneId={focusedPaneId}
            onActivate={onActivate}
            onSplit={onSplit}
            onClose={onClose}
          />
        </div>
        <div
          className={`shrink-0 border-line ${
            node.direction === "vertical" ? "border-l" : "border-t"
          }`}
          aria-hidden
        />
        <div className="flex min-h-0 min-w-0 flex-1">
          <PaneNodeView
            node={node.second}
            focusedPaneId={focusedPaneId}
            onActivate={onActivate}
            onSplit={onSplit}
            onClose={onClose}
          />
        </div>
      </div>
    );
  }

  return (
    <SessionLeaf
      paneId={node.id}
      sessionKey={node.sessionKey}
      focused={node.id === focusedPaneId}
      multiple={paneLeaves(useUi.getState().layout).length > 1}
      onActivate={() => onActivate(node.id)}
      onSplit={onSplit}
      onClose={() => onClose(node.id)}
    />
  );
}

function SessionLeaf({
  paneId,
  sessionKey,
  focused,
  multiple,
  onActivate,
  onSplit,
  onClose,
}: {
  paneId: string;
  sessionKey: string | null;
  focused: boolean;
  multiple: boolean;
  onActivate: () => void;
  onSplit: (direction: SplitDirection) => void;
  onClose: () => void;
}) {
  const [zone, setZone] = useState<DropZone | null>(null);
  const session = useStore((s) => s.sessions.find((item) => item.key === sessionKey));

  const zoneFor = (event: React.DragEvent<HTMLElement>) =>
    dropZoneAt(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY);

  return (
    <section
      aria-label={session ? `Session pane: ${session.title}` : "Empty session pane"}
      data-pane={paneId}
      data-focused={focused || undefined}
      className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col"
      onPointerDownCapture={onActivate}
      onFocusCapture={onActivate}
      onDragOver={(event) => {
        if (!event.dataTransfer.types.includes(SESSION_DRAG_TYPE)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setZone(zoneFor(event));
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setZone(null);
      }}
      onDrop={(event) => {
        const key = event.dataTransfer.getData(SESSION_DRAG_TYPE);
        if (!key) return;
        event.preventDefault();
        const droppedAt = zone ?? zoneFor(event);
        setZone(null);
        useUi.getState().dropSession(key, paneId, droppedAt);
        useStore.setState({ activeKey: key });
      }}
    >
      {sessionKey ? (
        <ChatPane
          sessionKey={sessionKey}
          focused={focused}
          onSplit={onSplit}
          onClose={onClose}
          canClose={multiple}
        />
      ) : (
        <EmptyPane paneId={paneId} onActivate={onActivate} canClose={multiple} onClose={onClose} />
      )}
      {zone && <DropIndicator zone={zone} empty={!sessionKey} />}
    </section>
  );
}

function EmptyPane({
  paneId,
  onActivate,
  canClose,
  onClose,
}: {
  paneId: string;
  onActivate: () => void;
  canClose: boolean;
  onClose: () => void;
}) {
  const projects = useStore((s) => s.projects);
  const show = useUi((s) => s.open);

  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
      {canClose && (
        <Button
          variant="ghost"
          size="sm"
          icon="x"
          iconOnly
          className="absolute top-2 right-2"
          aria-label="Close empty pane"
          title="Close pane"
          onClick={onClose}
        />
      )}
      <LoopMark size={72} />
      <div>
        <h1 className="text-title">Session pane</h1>
        <p className="mt-1 text-ink-muted">Select a session, or start a lane in a project.</p>
      </div>
      <div className="flex flex-wrap justify-center gap-2">
        <Button
          variant="secondary"
          icon="session"
          onClick={() => {
            onActivate();
            show({ type: "paneSession", paneId });
          }}
        >
          Select session
        </Button>
        <Button
          variant="primary"
          icon="plus"
          onClick={() => {
            onActivate();
            show({ type: "newLane", projectId: projects[0]?.id ?? 0 });
          }}
        >
          New lane
        </Button>
      </div>
    </main>
  );
}

function DropIndicator({ zone, empty }: { zone: DropZone; empty: boolean }) {
  const position =
    zone === "left"
      ? "inset-y-3 left-3 right-1/2"
      : zone === "right"
        ? "inset-y-3 left-1/2 right-3"
        : zone === "top"
          ? "inset-x-3 top-3 bottom-1/2"
          : zone === "bottom"
            ? "inset-x-3 top-1/2 bottom-3"
            : "inset-3";
  const label = empty
    ? "Drop session here"
    : zone === "center"
      ? "Drop session in this pane"
      : `Drop to split ${zone}`;
  return (
    <div
      className={`pointer-events-none absolute ${position} z-30 flex items-center justify-center rounded-md border border-focus bg-selection/80 p-3 text-body-strong text-ink`}
    >
      <Icon name={zone === "center" ? "session" : "plus"} size={16} />
      <span className="ml-2">{label}</span>
    </div>
  );
}
