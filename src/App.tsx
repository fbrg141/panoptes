import { Button, Icon } from "@panoptes/design-system";
import { useEffect } from "react";
import { api } from "./api";
import { ChatPane } from "./components/ChatPane";
import { NewLaneDialog } from "./components/NewLaneDialog";
import { NewSessionDialog } from "./components/NewSessionDialog";
import { SearchPalette } from "./components/SearchPalette";
import { ShortcutsDialog } from "./components/ShortcutsDialog";
import { Sidebar } from "./components/Sidebar";
import { pickProjectFolder } from "./pickProject";
import { shortcutFor } from "./shortcuts";
import { useStore } from "./store";
import { useUi } from "./ui";
import { nextZoom, zoomCommand } from "./zoom";

export default function App() {
  const init = useStore((s) => s.init);
  const info = useStore((s) => s.info);
  const error = useStore((s) => s.error);
  const dismissError = useStore((s) => s.dismissError);
  const overlay = useUi((s) => s.overlay);

  useEffect(() => {
    void init();
  }, [init]);

  // ⌘+/- zooms the whole window, ⌘0 resets it.
  useEffect(() => {
    let zoom = 1;
    const onKey = (e: KeyboardEvent) => {
      const command = zoomCommand(e);
      if (!command) return;
      e.preventDefault();
      zoom = nextZoom(zoom, command);
      void api.setZoom(zoom);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The window shortcuts: ⌘P searches, ⌘T starts a session, ⌘N adds a
  // project, ⌘/ shows the cheatsheet. An open overlay keeps the keyboard, so
  // a second shortcut does not yank it away from under the user.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const id = shortcutFor(e);
      if (!id || useUi.getState().overlay) return;
      e.preventDefault();
      if (id === "newProject") {
        void pickProjectFolder();
        return;
      }
      useUi
        .getState()
        .open(
          id === "search"
            ? { type: "search" }
            : id === "newSession"
              ? { type: "newSession" }
              : { type: "shortcuts" },
        );
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pi = info?.harnesses.find((h) => h.id === "pi");
  const warning =
    info?.errors[0] ??
    (pi && !pi.supported ? `Pi ${pi.version} is not a supported version. Expect problems.` : null);

  return (
    <div className="flex h-full flex-col bg-bg text-ink">
      {warning && (
        <div className="flex items-center gap-2 border-b border-line bg-waiting-soft px-4 py-1 text-waiting">
          <Icon name="circle-alert" size={14} />
          <span>{warning}</span>
        </div>
      )}
      {error && (
        <div
          role="alert"
          className="flex items-center gap-2 border-b border-line bg-error-soft px-4 py-1 text-error"
        >
          <Icon name="circle-alert" size={14} />
          <span className="flex-1">{error}</span>
          <Button variant="ghost" size="sm" onClick={dismissError}>
            Dismiss
          </Button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <ChatPane />
      </div>
      {overlay?.type === "search" && <SearchPalette />}
      {overlay?.type === "newSession" && <NewSessionDialog />}
      {overlay?.type === "shortcuts" && <ShortcutsDialog />}
      {overlay?.type === "newLane" && <NewLaneDialog projectId={overlay.projectId} />}
    </div>
  );
}
