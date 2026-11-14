/** A vertical split puts panes side by side; a horizontal split stacks them. */
export type SplitDirection = "vertical" | "horizontal";
export type DropZone = "left" | "right" | "top" | "bottom" | "center";

export interface PaneLeaf {
  type: "leaf";
  id: string;
  sessionKey: string | null;
}

export interface PaneSplit {
  type: "split";
  id: string;
  direction: SplitDirection;
  first: PaneNode;
  second: PaneNode;
}

export type PaneNode = PaneLeaf | PaneSplit;

let nextId = 0;
const id = (kind: string) => `${kind}-${++nextId}`;

export function singlePane(sessionKey: string | null = null): PaneLeaf {
  return { type: "leaf", id: id("pane"), sessionKey };
}

export function paneLeaves(node: PaneNode): PaneLeaf[] {
  return node.type === "leaf" ? [node] : [...paneLeaves(node.first), ...paneLeaves(node.second)];
}

export function findPane(node: PaneNode, paneId: string): PaneLeaf | null {
  return paneLeaves(node).find((pane) => pane.id === paneId) ?? null;
}

export function sessionPane(node: PaneNode, sessionKey: string): PaneLeaf | null {
  return paneLeaves(node).find((pane) => pane.sessionKey === sessionKey) ?? null;
}

function replace(
  node: PaneNode,
  paneId: string,
  replacement: (pane: PaneLeaf) => PaneNode,
): PaneNode {
  if (node.type === "leaf") return node.id === paneId ? replacement(node) : node;
  return {
    ...node,
    first: replace(node.first, paneId, replacement),
    second: replace(node.second, paneId, replacement),
  };
}

/** Splits a leaf, keeping its session first and returning the new empty leaf's id. */
export function splitPane(
  node: PaneNode,
  paneId: string,
  direction: SplitDirection,
): { layout: PaneNode; newPaneId: string | null } {
  const newPane = singlePane();
  let split = false;
  const layout = replace(node, paneId, (pane) => {
    split = true;
    return {
      type: "split",
      id: id("split"),
      direction,
      first: pane,
      second: newPane,
    };
  });
  return { layout, newPaneId: split ? newPane.id : null };
}

/** Removes a leaf and collapses its parent. The last leaf becomes empty. */
export function closePane(node: PaneNode, paneId: string): PaneNode {
  if (node.type === "leaf") {
    return node.id === paneId ? { ...node, sessionKey: null } : node;
  }
  if (node.first.type === "leaf" && node.first.id === paneId) return node.second;
  if (node.second.type === "leaf" && node.second.id === paneId) return node.first;
  return {
    ...node,
    first: closePane(node.first, paneId),
    second: closePane(node.second, paneId),
  };
}

/**
 * Shows a session once. Picking one already visible focuses its pane; an empty
 * destination is closed rather than left behind.
 */
export function placeSession(
  node: PaneNode,
  paneId: string,
  sessionKey: string,
): { layout: PaneNode; focusedPaneId: string } {
  const existing = sessionPane(node, sessionKey);
  const destination = findPane(node, paneId);
  if (!destination) {
    return {
      layout: node,
      focusedPaneId: existing?.id ?? paneLeaves(node)[0]?.id ?? paneId,
    };
  }
  if (existing) {
    const layout =
      existing.id !== paneId && destination.sessionKey === null ? closePane(node, paneId) : node;
    return { layout, focusedPaneId: existing.id };
  }
  return {
    layout: replace(node, paneId, (pane) => ({ ...pane, sessionKey })),
    focusedPaneId: paneId,
  };
}

/**
 * Drops a session from the sidebar or another pane. Edge drops split the
 * destination. A centre drop fills an empty pane, replaces an unrepresented
 * session, or swaps two panes that are already on screen.
 */
export function dropSession(
  node: PaneNode,
  sessionKey: string,
  paneId: string,
  zone: DropZone,
): { layout: PaneNode; focusedPaneId: string } {
  const destination = findPane(node, paneId);
  if (!destination) return { layout: node, focusedPaneId: paneId };
  const source = sessionPane(node, sessionKey);
  if (source?.id === paneId) return { layout: node, focusedPaneId: paneId };

  if (zone === "center" || destination.sessionKey === null) {
    if (!source) {
      return {
        layout: replace(node, paneId, (pane) => ({ ...pane, sessionKey })),
        focusedPaneId: paneId,
      };
    }
    if (destination.sessionKey === null) {
      const moved = replace(node, paneId, (pane) => ({ ...pane, sessionKey }));
      return { layout: closePane(moved, source.id), focusedPaneId: paneId };
    }
    return {
      layout: replace(
        replace(node, paneId, (pane) => ({ ...pane, sessionKey })),
        source.id,
        (pane) => ({ ...pane, sessionKey: destination.sessionKey }),
      ),
      focusedPaneId: paneId,
    };
  }

  const dragged = singlePane(sessionKey);
  const draggedFirst = zone === "left" || zone === "top";
  const direction: SplitDirection = zone === "left" || zone === "right" ? "vertical" : "horizontal";
  let layout = replace(node, paneId, (pane) => ({
    type: "split",
    id: id("split"),
    direction,
    first: draggedFirst ? dragged : pane,
    second: draggedFirst ? pane : dragged,
  }));
  // Split first so the destination still exists when the old leaf is removed.
  if (source) layout = closePane(layout, source.id);
  return { layout, focusedPaneId: dragged.id };
}

/** The drop zone under a pointer, with the outer quarter reserved for splits. */
export function dropZoneAt(
  bounds: Pick<DOMRect, "left" | "top" | "width" | "height">,
  x: number,
  y: number,
): DropZone {
  const px = bounds.width === 0 ? 0.5 : (x - bounds.left) / bounds.width;
  const py = bounds.height === 0 ? 0.5 : (y - bounds.top) / bounds.height;
  if (px < 0.25) return "left";
  if (px > 0.75) return "right";
  if (py < 0.25) return "top";
  if (py > 0.75) return "bottom";
  return "center";
}

export const SESSION_DRAG_TYPE = "application/x-panoptes-session";
