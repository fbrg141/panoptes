import { Button, Icon } from "@panoptes/design-system";
import { useEffect } from "react";
import { api } from "./api";
import { CleanupDialog } from "./components/CleanupDialog";
import { CrashNotice, ErrorBoundary } from "./components/ErrorBoundary";
import { NewLaneDialog } from "./components/NewLaneDialog";
import { ResumeSessionDialog } from "./components/ResumeSessionDialog";
import { SearchPalette } from "./components/SearchPalette";
import { SessionPicker } from "./components/SessionPicker";
import { SessionWorkspace } from "./components/SessionWorkspace";
import { SettingsDialog } from "./components/SettingsDialog";
import { ShortcutsDialog } from "./components/ShortcutsDialog";
import { Sidebar } from "./components/Sidebar";
import { StartupGate } from "./components/StartupGate";
import { pickProjectFolder } from "./pickProject";
import { shortcutFor } from "./shortcuts";
import { useStore } from "./store";
import { useUi } from "./ui";
import { nextZoom, zoomCommand } from "./zoom";

export default function App() {
  const init = useStore((s) => s.init);
  const startupStatus = useStore((s) => s.startupStatus);
  const startupError = useStore((s) => s.startupError);
  const info = useStore((s) => s.info);
  const error = useStore((s) => s.error);
  const dismissError = useStore((s) => s.dismissError);
  const projects = useStore((s) => s.projects);
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

  // The window shortcuts: ⌘P searches, ⌘T starts a lane from a prompt,
  // ⌘N adds a project, ⌘D splits a pane, and ⌘/ shows the cheatsheet. An open overlay keeps the
  // keyboard, so a second shortcut does not yank it away from under the user.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const editing =
        e.target instanceof HTMLElement &&
        (e.target.isContentEditable || e.target.closest("input, textarea") !== null);
      const id = shortcutFor(e, editing);
      if (!id || useUi.getState().overlay) return;
      e.preventDefault();
      if (id === "newProject") {
        void pickProjectFolder();
        return;
      }
      if (id === "newLaneSession") {
        const state = useStore.getState();
        const laneId = state.sessions.find((session) => session.key === state.activeKey)?.laneId;
        if (laneId !== null && laneId !== undefined) void state.newLaneSession(laneId);
        return;
      }
      if (id === "splitVertical" || id === "splitHorizontal") {
        const paneId = useUi
          .getState()
          .splitFocused(id === "splitVertical" ? "vertical" : "horizontal");
        if (paneId) useStore.setState({ activeKey: null });
        return;
      }
      const overlay =
        id === "search"
          ? { type: "search" as const }
          : id === "newSession"
            ? { type: "newLane" as const, projectId: projects[0]?.id ?? 0 }
            : { type: "shortcuts" as const };
      useUi.getState().open(overlay);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [projects]);

  const pi = info?.harnesses.find((h) => h.id === "pi");
  const warning =
    info?.errors[0] ??
    (pi && !pi.supported ? `Pi ${pi.version} is not a supported version. Expect problems.` : null);

  return (
    <StartupGate status={startupStatus} error={startupError} onRetry={() => void init()}>
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
          <ErrorBoundary
            label="sidebar"
            fallback={({ error, reset }) => (
              <aside className="flex w-80 shrink-0 border-r border-line bg-surface">
                <CrashNotice title="The sidebar hit an error" error={error}>
                  <Button variant="secondary" size="sm" onClick={reset}>
                    Try again
                  </Button>
                </CrashNotice>
              </aside>
            )}
          >
            <Sidebar />
          </ErrorBoundary>
          <SessionWorkspace />
        </div>
        {/* A crashed dialog closes on its own; the window behind it keeps working. */}
        <ErrorBoundary
          label="dialog"
          resetKey={overlay}
          fallback={({ error }) => <OverlayCrash error={error} />}
        >
          {overlay?.type === "search" && <SearchPalette />}
          {overlay?.type === "shortcuts" && <ShortcutsDialog />}
          {overlay?.type === "paneSession" && <SessionPicker paneId={overlay.paneId} />}
          {overlay?.type === "settings" && <SettingsDialog />}
          {overlay?.type === "cleanup" && <CleanupDialog projectId={overlay.projectId} />}
          {overlay?.type === "newLane" && (
            <NewLaneDialog projectId={overlay.projectId} prompt={overlay.prompt} />
          )}
          {overlay?.type === "resumeSession" && (
            <ResumeSessionDialog laneId={overlay.laneId} sessionId={overlay.sessionId} />
          )}
        </ErrorBoundary>
      </div>
    </StartupGate>
  );
}

function OverlayCrash({ error }: { error: Error }) {
  const close = useUi((s) => s.close);
  useEffect(() => {
    useStore.setState({ error: `That dialog hit an error and was closed: ${error.message}` });
    close();
  }, [error, close]);
  return null;
}
