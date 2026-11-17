import { create } from "zustand";
import {
  closePane,
  type DropZone,
  dropSession,
  findPane,
  type PaneNode,
  paneLeaves,
  placeSession,
  type SplitDirection,
  singlePane,
  splitPane,
} from "./panes";

/**
 * The one overlay the window shows at a time: the search palette, the new
 * lane form, settings, the cheatsheet, or a lane form opened for a project. Global
 * shortcuts, the sidebar and the palette all open things through here, so a
 * second overlay replaces the first instead of stacking.
 */
export type Overlay =
  | { type: "search" }
  | { type: "newProject" }
  | { type: "shortcuts" }
  | { type: "paneSession"; paneId: string }
  | { type: "settings" }
  | { type: "cleanup"; projectId: number }
  | { type: "newLane"; projectId: number; prompt?: string };

const initialPane = singlePane();

export interface UiState {
  overlay: Overlay | null;
  /** The window's tiling layout; sessions and processes remain in the store. */
  layout: PaneNode;
  focusedPaneId: string;
  open: (overlay: Overlay) => void;
  close: () => void;
  focusPane: (paneId: string) => void;
  splitFocused: (direction: SplitDirection) => string | null;
  showSession: (sessionKey: string, paneId?: string) => void;
  closePane: (paneId: string) => void;
  dropSession: (sessionKey: string, paneId: string, zone: DropZone) => void;
}

export const useUi = create<UiState>((set, get) => ({
  overlay: null,
  layout: initialPane,
  focusedPaneId: initialPane.id,
  open: (overlay) => set({ overlay }),
  close: () => set({ overlay: null }),
  focusPane: (focusedPaneId) => {
    if (findPane(get().layout, focusedPaneId)) set({ focusedPaneId });
  },
  splitFocused: (direction) => {
    const result = splitPane(get().layout, get().focusedPaneId, direction);
    if (!result.newPaneId) return null;
    const newPaneId = result.newPaneId;
    set({
      layout: result.layout,
      focusedPaneId: newPaneId,
      overlay: { type: "paneSession", paneId: newPaneId },
    });
    return newPaneId;
  },
  showSession: (sessionKey, paneId = get().focusedPaneId) => {
    const result = placeSession(get().layout, paneId, sessionKey);
    set({ layout: result.layout, focusedPaneId: result.focusedPaneId });
  },
  closePane: (paneId) =>
    set((state) => {
      const layout = closePane(state.layout, paneId);
      const focusedPaneId = findPane(layout, state.focusedPaneId)
        ? state.focusedPaneId
        : (paneLeaves(layout)[0]?.id ?? paneId);
      return { layout, focusedPaneId };
    }),
  dropSession: (sessionKey, paneId, zone) => {
    const result = dropSession(get().layout, sessionKey, paneId, zone);
    set({ layout: result.layout, focusedPaneId: result.focusedPaneId });
  },
}));
