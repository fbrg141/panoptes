import { create } from "zustand";

/**
 * The one overlay the window shows at a time: the search palette, the new
 * session form, the cheatsheet, or a lane form opened for a project. Global
 * shortcuts, the sidebar and the palette all open things through here, so a
 * second overlay replaces the first instead of stacking.
 */
export type Overlay =
  | { type: "search" }
  | { type: "newSession" }
  | { type: "newProject" }
  | { type: "shortcuts" }
  | { type: "newLane"; projectId: number };

export interface UiState {
  overlay: Overlay | null;
  open: (overlay: Overlay) => void;
  close: () => void;
}

export const useUi = create<UiState>((set) => ({
  overlay: null,
  open: (overlay) => set({ overlay }),
  close: () => set({ overlay: null }),
}));
