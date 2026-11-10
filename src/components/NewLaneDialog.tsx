import { Button, Checkbox, Dialog, Input } from "@panoptes/design-system";
import { useEffect, useState } from "react";
import { useStore } from "../store";
import { useUi } from "../ui";

/**
 * The new lane form, opened from a project's row menu, the search palette or
 * a shortcut. It lives outside the sidebar so every entry point shares it.
 */
export function NewLaneDialog({ projectId }: { projectId: number }) {
  const project = useStore((s) => s.projects.find((item) => item.id === projectId));
  const createLane = useStore((s) => s.createLane);
  const createSession = useStore((s) => s.create);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const close = useUi((s) => s.close);
  const [task, setTask] = useState("");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [startSession, setStartSession] = useState(true);
  const [busy, setBusy] = useState(false);

  // The project can be removed while the form is open.
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
      if (startSession) await createSession(lane.id);
    }
  };

  const cancel = () => {
    if (busy) void cancelLaneCreation(project.id);
    close();
  };

  return (
    <Dialog
      className="overlay-panel"
      title={`New lane in ${project.name}`}
      onClose={cancel}
      actions={
        <>
          <Button variant="secondary" onClick={cancel}>
            Cancel
          </Button>
          <Button
            type="submit"
            form="new-lane"
            variant="primary"
            kbd={busy ? undefined : ["↵"]}
            disabled={busy || !task.trim()}
          >
            {busy ? "Creating…" : "Create lane"}
          </Button>
        </>
      }
    >
      <form
        id="new-lane"
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Input
          label="Task"
          hint="Names the lane. It gets its own worktree, branch and write lock."
          placeholder="Fix login redirects"
          aria-label="Lane task"
          value={task}
          onChange={(event) => setTask(event.target.value)}
          disabled={busy}
          autoFocus
        />
        <Checkbox
          label="Use the main checkout"
          hint="Share the project's checkout: no worktree, copied files or setup command. Archiving the lane never removes it."
          checked={mainCheckout}
          onChange={(event) => setMainCheckout(event.target.checked)}
          disabled={busy}
        />
        <Checkbox
          label="Start a session in this lane"
          hint="Open a session in the lane as soon as it is ready."
          checked={startSession}
          onChange={(event) => setStartSession(event.target.checked)}
          disabled={busy}
        />
      </form>
    </Dialog>
  );
}
