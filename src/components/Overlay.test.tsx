// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../store";
import { useUi } from "../ui";
import { ModalDialog, Overlay } from "./Overlay";
import { SearchPalette } from "./SearchPalette";
import { ShortcutsDialog } from "./ShortcutsDialog";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let opener: HTMLButtonElement;

beforeEach(() => {
  opener = document.createElement("button");
  document.body.append(opener);
  opener.focus();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  useStore.setState({ sessions: [], projects: [], lanes: [], activeKey: null });
});

afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
  useUi.setState({ overlay: null });
});

function press(key: string, shiftKey = false) {
  act(() => {
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }),
    );
  });
}

function App() {
  const overlay = useUi((s) => s.overlay);
  return (
    <>
      {overlay?.type === "search" && <SearchPalette />}
      {overlay?.type === "shortcuts" && <ShortcutsDialog />}
    </>
  );
}

function Confirm({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return (
    <ModalDialog
      title="Delete lane?"
      onClose={onClose}
      actions={<button type="button">Delete permanently</button>}
    >
      <p>Gone for good.</p>
    </ModalDialog>
  );
}

describe("modal overlays", () => {
  it("gives the search palette a modal dialog that Escape closes", () => {
    useUi.setState({ overlay: { type: "search" } });
    act(() => root.render(<App />));
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(dialog?.getAttribute("aria-label")).toBe("Search");
    expect(dialog?.contains(document.activeElement)).toBe(true);
    // The palette has one field: Tab must not leave it for the window behind.
    press("Tab");
    expect(dialog?.contains(document.activeElement)).toBe(true);
    press("Escape");
    expect(useUi.getState().overlay).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it("closes the shortcuts dialog on Escape even after focus slipped out", () => {
    useUi.setState({ overlay: { type: "shortcuts" } });
    act(() => root.render(<App />));
    (document.activeElement as HTMLElement).blur();
    press("Escape");
    expect(useUi.getState().overlay).toBeNull();
  });

  it("keeps an undismissable panel open on Escape", () => {
    act(() =>
      root.render(
        <Overlay label="Busy" width={400}>
          <button type="button">Wait</button>
        </Overlay>,
      ),
    );
    press("Escape");
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it("holds the keyboard in a confirmation dialog and closes it once on Escape", () => {
    const onClose = vi.fn();
    act(() => root.render(<Confirm open onClose={onClose} />));
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog?.contains(document.activeElement)).toBe(true);
    for (let i = 0; i < 4; i++) {
      press("Tab");
      expect(dialog?.contains(document.activeElement)).toBe(true);
    }
    press("Escape");
    expect(onClose).toHaveBeenCalledOnce();
    act(() => root.render(<Confirm open={false} onClose={onClose} />));
    expect(document.activeElement).toBe(opener);
  });
});
