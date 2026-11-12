import { Button, Icon } from "@panoptes/design-system";
import { useState } from "react";
import { pickProjectFolder } from "../pickProject";
import { useStore } from "../store";
import { useUi } from "../ui";
import { OptionToggle, Overlay, OverlayFooter, OverlayHeader } from "./Overlay";

/**
 * The project + flow: one prompt creates a lane and its only session. The
 * prompt names the lane and becomes its first message.
 */
export function NewLaneDialog({ projectId, prompt }: { projectId: number; prompt?: string }) {
  const project = useStore((s) => s.projects.find((item) => item.id === projectId));
  const createFromPrompt = useStore((s) => s.createFromPrompt);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const close = useUi((s) => s.close);
  const [task, setTask] = useState(prompt ?? "");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [busy, setBusy] = useState(false);

  if (!project) {
    return (
      <Overlay label="New lane" width={480} onClose={close}>
        <OverlayHeader onClose={close}>
          <h2 className="text-heading">New lane</h2>
        </OverlayHeader>
        <div className="flex flex-col items-center gap-3 px-6 pt-4 pb-8 text-center">
          <Icon name="folder-open" size={24} className="text-ink-muted" />
          <p className="max-w-80 text-ink-muted">
            Lanes belong to projects. Add a git repository to begin.
          </p>
          <Button variant="primary" autoFocus onClick={() => void pickProjectFolder()}>
            Add a project
          </Button>
        </div>
      </Overlay>
    );
  }

  const submit = async () => {
    const text = task.trim();
    if (!text || busy) return;
    setBusy(true);
    const created = await createFromPrompt(project.id, text, mainCheckout);
    if (created) {
      close();
    } else {
      setBusy(false);
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
            aria-label="First prompt"
            aria-describedby="new-lane-help"
            placeholder="What should Pi do? e.g. Fix login redirects"
            value={task}
            onChange={(event) => setTask(event.target.value)}
            disabled={busy}
            // biome-ignore lint/a11y/noAutofocus: the prompt is what this panel asks for
            autoFocus
            className="menu-search h-9 w-full bg-transparent text-heading font-medium text-ink placeholder:font-normal placeholder:text-ink-faint"
          />
          <p id="new-lane-help" className="text-label font-normal text-ink-muted">
            {mainCheckout
              ? "Shares the project's own checkout: no worktree, copied files or setup command. Archiving the lane never removes it."
              : `Gets its own worktree and branch from ${project.baseBranch}. Your prompt starts the lane's one session.`}
          </p>
        </div>

        <OverlayFooter>
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
