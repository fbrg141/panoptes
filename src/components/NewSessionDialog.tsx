import { Button, Dialog, Input, Select } from "@panoptes/design-system";
import { useEffect, useMemo, useState } from "react";
import { useStore } from "../store";
import { useUi } from "../ui";

/**
 * The ⌘T form: pick a lane, optionally a model and thinking level, and the
 * first prompt. The session is recorded, configured and started in one go;
 * anything left blank keeps the harness's own default.
 */
export function NewSessionDialog() {
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const activeKey = useStore((s) => s.activeKey);
  const modelsByKey = useStore((s) => s.models);
  const settingsByKey = useStore((s) => s.settings);
  const create = useStore((s) => s.create);
  const loadModels = useStore((s) => s.loadModels);
  const close = useUi((s) => s.close);
  const activeLanes = useMemo(() => lanes.filter((lane) => lane.state === "active"), [lanes]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [laneId, setLaneId] = useState<number | null>(null);
  const [model, setModel] = useState("");
  const [thinking, setThinking] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  // The user picked a model or a level, but there is no open session to read
  // either from: say so rather than looping on a failing load.
  const [choicesUnavailable, setChoicesUnavailable] = useState(false);

  // Model and thinking choices come from an open session: listing them needs
  // a harness process. Ask for the list if none is loaded yet.
  const models = useMemo(
    () =>
      (activeKey && modelsByKey[activeKey]) ??
      Object.values(modelsByKey).find((list) => list.length > 0),
    [activeKey, modelsByKey],
  );
  const levels = useMemo(() => {
    const settings =
      (activeKey ? settingsByKey[activeKey] : undefined) ??
      Object.values(settingsByKey).find((item) => item && item.thinkingLevels.length > 0);
    return settings?.thinkingLevels ?? [];
  }, [activeKey, settingsByKey]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: asks once, when the form opens
  useEffect(() => {
    if (!models) void loadModels();
  }, []);

  // Default to the first project that has a lane to work in, and keep the
  // choice valid as projects and lanes come and go.
  const projectLanes = (id: number | null) => activeLanes.filter((lane) => lane.projectId === id);
  useEffect(() => {
    const stillThere =
      projectId !== null && lanes.some((lane) => lane.state === "active" && lane.id === laneId);
    if (stillThere) return;
    const active = lanes.filter((lane) => lane.state === "active");
    const project =
      projects.find((item) => active.some((lane) => lane.projectId === item.id)) ??
      projects[0] ??
      null;
    setProjectId(project?.id ?? null);
    setLaneId(project ? (active.find((lane) => lane.projectId === project.id)?.id ?? null) : null);
  }, [projects, lanes, projectId, laneId]);

  if (activeLanes.length === 0) {
    return (
      <Dialog title="New session" onClose={close}>
        <p className="text-ink-muted">
          Sessions live in lanes. Add a project and create a lane to work in first.
        </p>
      </Dialog>
    );
  }

  const loadChoices = async () => {
    // loadModels reports failure on the window's error banner; detect it by
    // watching that error rather than guessing from the models list.
    const before = useStore.getState().error;
    await loadModels();
    if (useStore.getState().error !== before) setChoicesUnavailable(true);
  };

  const unavailableNote = choicesUnavailable && (
    <p className="text-caption text-ink-muted">
      Model and thinking choices need an open session to read them from. Start this session to pick
      them in the composer.
    </p>
  );

  const submit = async () => {
    if (laneId === null || busy) return;
    setBusy(true);
    await create(laneId, {
      model: model || undefined,
      thinking: thinking || undefined,
      prompt: prompt.trim() || undefined,
    });
    setBusy(false);
    close();
  };

  return (
    <Dialog
      title="New session"
      onClose={busy ? undefined : close}
      actions={
        <>
          <Button variant="secondary" disabled={busy} onClick={close}>
            Cancel
          </Button>
          <Button type="submit" form="new-session" disabled={busy || laneId === null}>
            {busy ? "Starting…" : "Start session"}
          </Button>
        </>
      }
    >
      <form
        id="new-session"
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Select
          label="Project"
          icon="folder"
          autoFocus
          value={projectId ?? ""}
          options={projects.map((project) => ({
            value: String(project.id),
            label: project.name,
          }))}
          onChange={(event) => {
            const id = Number(event.target.value);
            setProjectId(id);
            setLaneId(projectLanes(id)[0]?.id ?? null);
          }}
          disabled={busy}
        />
        <Select
          label="Lane"
          icon="git-branch"
          hint="The session runs in the lane's worktree, behind its write lock."
          value={laneId ?? ""}
          options={projectLanes(projectId).map((lane) => ({
            value: String(lane.id),
            label: lane.task,
          }))}
          onChange={(event) => setLaneId(Number(event.target.value))}
          disabled={busy || projectLanes(projectId).length === 0}
        />
        <Select
          label="Model"
          icon="agent"
          hint="Blank keeps the harness's default."
          value={model}
          options={
            models
              ? [
                  { value: "", label: "Harness default" },
                  ...models.map((item) => ({
                    value: item.id,
                    label: `${item.provider} · ${item.name}`,
                  })),
                ]
              : [{ value: "", label: "Loading models…", disabled: true }]
          }
          onChange={(event) => {
            if (event.target.value && !models) void loadChoices();
            setModel(event.target.value);
          }}
          disabled={busy || !models}
        />
        {levels.length > 0 && (
          <Select
            label="Thinking level"
            icon="circle-dot"
            hint="Levels of the currently chosen model; blank keeps its default."
            value={thinking}
            options={[{ value: "", label: "Model default" }, ...levels]}
            onChange={(event) => setThinking(event.target.value)}
            disabled={busy}
          />
        )}
        {unavailableNote}
        <Input
          label="First prompt"
          placeholder="What should the agent do first?"
          aria-label="First prompt"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          disabled={busy}
        />
      </form>
    </Dialog>
  );
}
