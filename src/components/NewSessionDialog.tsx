import { Button, Dialog, Input, Select } from "@panoptes/design-system";
import { useEffect, useMemo, useState } from "react";
import { pickProjectFolder } from "../pickProject";
import { useStore } from "../store";
import { useUi } from "../ui";

/**
 * The ⌘T form: pick a lane, optionally a model and thinking level, and the
 * first prompt. The session is recorded, configured and started in one go;
 * anything left blank keeps the harness's own default.
 */
export function NewSessionDialog() {
  const projects = useStore((s) => s.projects);
  const sessions = useStore((s) => s.sessions);
  const lanes = useStore((s) => s.lanes);
  const activeKey = useStore((s) => s.activeKey);
  const modelsByKey = useStore((s) => s.models);
  const settingsByKey = useStore((s) => s.settings);
  const create = useStore((s) => s.create);
  const loadModels = useStore((s) => s.loadModels);
  const close = useUi((s) => s.close);
  const show = useUi((s) => s.open);
  const activeLanes = useMemo(() => lanes.filter((lane) => lane.state === "active"), [lanes]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [laneId, setLaneId] = useState<number | null>(null);
  const [model, setModel] = useState("");
  const [thinking, setThinking] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);

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
    if (!models && activeKey) void loadModels();
  }, []);
  // Listing models needs an open session; without one the harness default is
  // the only honest choice, and the composer offers the rest later.
  const modelsUnavailable = !models && !activeKey;

  // Default to the lane of the session on screen, else the first project
  // with a lane to work in, and keep the choice valid as projects and lanes
  // come and go. A project without lanes stays picked, with no lane.
  const projectLanes = (id: number | null) => activeLanes.filter((lane) => lane.projectId === id);
  useEffect(() => {
    const projectThere = projects.some((item) => item.id === projectId);
    const laneThere = laneId === null || activeLanes.some((lane) => lane.id === laneId);
    if (projectThere && laneThere) return;
    const current = sessions.find((session) => session.key === activeKey);
    const lane =
      activeLanes.find((item) => item.id === current?.laneId) ??
      activeLanes.find((item) => projects.some((project) => project.id === item.projectId));
    const project = projects.find((item) => item.id === lane?.projectId) ?? projects[0] ?? null;
    setProjectId(project?.id ?? null);
    setLaneId(lane && lane.projectId === project?.id ? lane.id : null);
  }, [projects, activeLanes, sessions, activeKey, projectId, laneId]);

  if (activeLanes.length === 0) {
    const first = projects[0];
    return (
      <Dialog
        className="overlay-panel"
        title="New session"
        onClose={close}
        actions={
          <>
            <Button variant="secondary" onClick={close}>
              Cancel
            </Button>
            <Button
              variant="primary"
              autoFocus
              onClick={() =>
                first ? show({ type: "newLane", projectId: first.id }) : void pickProjectFolder()
              }
            >
              {first ? `New lane in ${first.name}` : "Add a project"}
            </Button>
          </>
        }
      >
        <p>
          {first
            ? "Sessions run in lanes, and no lane is active yet. Create one to start working."
            : "Sessions run in lanes, which belong to projects. Add a git repository to begin."}
        </p>
      </Dialog>
    );
  }

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

  const lanesHere = projectLanes(projectId);
  const modelOptions = models
    ? [
        { value: "", label: "Harness default" },
        ...models.map((item) => ({ value: item.id, label: `${item.name} · ${item.provider}` })),
      ]
    : [{ value: "", label: modelsUnavailable ? "Harness default" : "Loading models…" }];

  return (
    <Dialog
      className="overlay-panel"
      title="New session"
      width={520}
      onClose={busy ? undefined : close}
      actions={
        <>
          <Button variant="secondary" disabled={busy} onClick={close}>
            Cancel
          </Button>
          <Button
            type="submit"
            form="new-session"
            variant="primary"
            kbd={busy ? undefined : ["↵"]}
            disabled={busy || laneId === null}
          >
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
        <div className="grid grid-cols-2 gap-3">
          <Select
            label="Project"
            icon="folder"
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
            value={laneId ?? ""}
            options={
              lanesHere.length > 0
                ? lanesHere.map((lane) => ({ value: String(lane.id), label: lane.task }))
                : [{ value: "", label: "No active lanes" }]
            }
            onChange={(event) => setLaneId(Number(event.target.value))}
            disabled={busy || lanesHere.length === 0}
          />
        </div>
        <Input
          label="First prompt"
          hint="Optional. The session runs in the lane's worktree, behind its write lock."
          placeholder="What should the agent do first?"
          aria-label="First prompt"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          disabled={busy}
          autoFocus
        />
        <div className={levels.length > 0 ? "grid grid-cols-2 gap-3" : ""}>
          <Select
            label="Model"
            icon="agent"
            hint={
              modelsUnavailable
                ? "Open a session to list models; you can switch in the composer."
                : undefined
            }
            value={model}
            options={modelOptions}
            onChange={(event) => setModel(event.target.value)}
            disabled={busy || !models}
          />
          {levels.length > 0 && (
            <Select
              label="Thinking"
              icon="circle-dot"
              value={thinking}
              options={[{ value: "", label: "Model default" }, ...levels]}
              onChange={(event) => setThinking(event.target.value)}
              disabled={busy}
            />
          )}
        </div>
      </form>
    </Dialog>
  );
}
