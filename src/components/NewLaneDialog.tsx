import { Button, Icon } from "@panoptes/design-system";
import { useEffect, useState } from "react";
import { useStore } from "../store";
import { useUi } from "../ui";
import { OptionToggle, Overlay, OverlayFooter, OverlayHeader } from "./Overlay";

/**
 * The new lane panel, opened from a project's row menu, the search palette,
 * or the new session panel (which hands over the prompt typed so far). The
 * task names the lane; the footer holds what happens around it.
 */
export function NewLaneDialog({ projectId, prompt }: { projectId: number; prompt?: string }) {
  const project = useStore((s) => s.projects.find((item) => item.id === projectId));
  const createLane = useStore((s) => s.createLane);
  const createSession = useStore((s) => s.create);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const close = useUi((s) => s.close);
  const [task, setTask] = useState("");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [startSession, setStartSession] = useState(true);
  const [busy, setBusy] = useState(false);

  // The project can be removed while the panel is open.
  useEffect(() => {
    if (!project) close();
  }, [project, close]);

  if (!project) return null;

  const submit = async () => {
    if (!task.trim() || busy) return;
    setBusy(true);
    const lane = await createLane(project.id, task, mainCheckout);
    setBusy(false);
    if (lane) {
      close();
      if (startSession) await createSession(lane.id, { prompt });
    }
  };

  const cancel = () => {
    if (busy) void cancelLaneCreation(project.id);
    close();
  };

  return (
    <Overlay label={`New lane in ${project.name}`} width={560} onClose={cancel}>
      <form
        className="flex min-h-0 flex-col"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <OverlayHeader onClose={cancel}>
          <span className="flex min-w-0 items-center gap-1.5 text-label font-normal text-ink-muted">
            <Icon name="folder" size={14} className="flex-none" />
            <span className="truncate">{project.name}</span>
          </span>
          <Icon name="chevron-right" size={12} className="flex-none text-ink-faint" />
          <h2 className="text-body-strong">New lane</h2>
        </OverlayHeader>

        <div className="flex flex-col gap-2 px-4 pt-1 pb-4">
          <input
            aria-label="Lane task"
            aria-describedby="new-lane-help"
            placeholder="What is this lane for? e.g. Fix login redirects"
            value={task}
            onChange={(event) => setTask(event.target.value)}
            disabled={busy}
            // biome-ignore lint/a11y/noAutofocus: the task is what this panel asks for
            autoFocus
            className="menu-search h-9 w-full bg-transparent text-heading font-medium text-ink placeholder:font-normal placeholder:text-ink-faint"
          />
          <p id="new-lane-help" className="text-label font-normal text-ink-muted">
            {mainCheckout
              ? "Shares the project's own checkout: no worktree, copied files or setup command. Archiving the lane never removes it."
              : `Gets its own worktree, branch and write lock, branched from ${project.baseBranch}.`}
          </p>
          {prompt && (
            <div className="mt-1 flex gap-2 rounded-sm border border-line bg-surface-sunken px-3 py-2">
              <Icon name="message-square" size={14} className="mt-0.5 flex-none text-ink-muted" />
              <p className="line-clamp-3 min-w-0 whitespace-pre-wrap text-ink">
                <span className="text-ink-muted">First prompt · </span>
                {prompt}
              </p>
            </div>
          )}
        </div>

        <OverlayFooter>
          <OptionToggle
            label="Start a session"
            title={
              prompt
                ? "Start a session with your prompt"
                : "Open a session in the lane once it is ready"
            }
            pressed={startSession}
            disabled={busy}
            onChange={setStartSession}
          />
          <OptionToggle
            label="Use the main checkout"
            title="Work in the project's own checkout instead of a new worktree"
            pressed={mainCheckout}
            disabled={busy}
            onChange={setMainCheckout}
          />
          <Button
            type="submit"
            variant="primary"
            kbd={busy ? undefined : ["↵"]}
            disabled={busy || !task.trim()}
            className="ml-auto"
          >
            {busy ? "Creating lane…" : "Create lane"}
          </Button>
        </OverlayFooter>
      </form>
    </Overlay>
  );
}
