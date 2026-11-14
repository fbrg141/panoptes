import { Button, Icon } from "@panoptes/design-system";
import { useEffect, useMemo, useRef, useState } from "react";
import { DEFAULT_CHOICE, modelGroups, shortModelId, thinkingGroups } from "../newSession";
import { pickProjectFolder } from "../pickProject";
import { useStore } from "../store";
import { useUi } from "../ui";
import { OptionToggle, Overlay, OverlayFooter, OverlayHeader } from "./Overlay";
import { Picker } from "./Picker";

/**
 * The project + flow: one prompt creates a lane and its only session. The
 * prompt names the lane and becomes its first message.
 */
export function NewLaneDialog({ projectId, prompt }: { projectId: number; prompt?: string }) {
  const project = useStore((s) => s.projects.find((item) => item.id === projectId));
  const createFromPrompt = useStore((s) => s.createFromPrompt);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const close = useUi((s) => s.close);
  const activeKey = useStore((s) => s.activeKey);
  const modelsByKey = useStore((s) => s.models);
  const settingsByKey = useStore((s) => s.settings);
  const loadModels = useStore((s) => s.loadModels);
  const [task, setTask] = useState(prompt ?? "");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [model, setModel] = useState(DEFAULT_CHOICE);
  const [thinking, setThinking] = useState(DEFAULT_CHOICE);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLInputElement>(null);

  // Model and thinking choices are read from an open session: listing them
  // needs a harness process. Without one, the harness default is the only
  // honest choice, and the composer offers the rest once the session runs.
  const models = useMemo(
    () =>
      (activeKey ? modelsByKey[activeKey] : undefined) ??
      Object.values(modelsByKey).find((list) => list.length > 0),
    [activeKey, modelsByKey],
  );
  const levels = useMemo(() => {
    const settings =
      (activeKey ? settingsByKey[activeKey] : undefined) ??
      Object.values(settingsByKey).find((item) => item && item.thinkingLevels.length > 0);
    return settings?.thinkingLevels ?? [];
  }, [activeKey, settingsByKey]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: asks once, when the panel opens
  useEffect(() => {
    if (!models && activeKey) void loadModels();
  }, []);

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

  const chosenModel = models?.find((item) => item.id === model);
  const focusTask = () => field.current?.focus();

  const submit = async () => {
    const text = task.trim();
    if (!text || busy) return;
    setBusy(true);
    const created = await createFromPrompt(project.id, text, mainCheckout, {
      model: model || undefined,
      thinking: thinking || undefined,
    });
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
            ref={field}
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
          <Picker
            label="Model"
            icon="agent"
            title={
              models
                ? undefined
                : activeKey
                  ? "Loading models"
                  : "Models are listed from an open session; switch in the composer once this one runs"
            }
            value={
              chosenModel ? (
                <span className="text-code-sm">{shortModelId(chosenModel)}</span>
              ) : (
                "Default model"
              )
            }
            valueText={chosenModel?.name ?? "Default model"}
            groups={models ? modelGroups(models, model) : []}
            emptyText="Loading models…"
            disabled={busy || !models}
            returnFocus={focusTask}
            onSelect={setModel}
          />
          {levels.length > 0 && (
            <Picker
              label="Thinking"
              icon="circle-dot"
              value={thinking ? `Thinking: ${thinking}` : "Default thinking"}
              groups={thinkingGroups(levels, thinking)}
              searchable={false}
              width={200}
              disabled={busy}
              returnFocus={focusTask}
              onSelect={setThinking}
            />
          )}
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
