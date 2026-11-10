import { Button, Icon, Kbd } from "@panoptes/design-system";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  DEFAULT_CHOICE,
  defaultTarget,
  firstLane,
  laneGroups,
  modelGroups,
  NEW_LANE,
  projectGroups,
  shortModelId,
  thinkingGroups,
} from "../newSession";
import { pickProjectFolder } from "../pickProject";
import { useStore } from "../store";
import { useUi } from "../ui";
import { Overlay, OverlayFooter, OverlayHeader } from "./Overlay";
import { Picker } from "./Picker";

/**
 * The ⌘T panel, shaped like the composer it leads to: where the session runs
 * on top, the first prompt as the body, and the model and thinking level as
 * chips beside the one action. Enter starts it, as Enter sends in the
 * composer; anything left alone keeps the harness's own default.
 */
export function NewSessionDialog() {
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const sessions = useStore((s) => s.sessions);
  const activeKey = useStore((s) => s.activeKey);
  const modelsByKey = useStore((s) => s.models);
  const settingsByKey = useStore((s) => s.settings);
  const create = useStore((s) => s.create);
  const loadModels = useStore((s) => s.loadModels);
  const close = useUi((s) => s.close);
  const show = useUi((s) => s.open);
  const [target, setTarget] = useState(() => defaultTarget(projects, lanes, sessions, activeKey));
  const [model, setModel] = useState(DEFAULT_CHOICE);
  const [thinking, setThinking] = useState(DEFAULT_CHOICE);
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);

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

  // Keep the choice valid as projects and lanes come and go.
  useEffect(() => {
    const projectThere = projects.some((item) => item.id === target.projectId);
    const laneThere =
      target.laneId === null ||
      lanes.some((lane) => lane.state === "active" && lane.id === target.laneId);
    if (!projectThere || !laneThere) setTarget(defaultTarget(projects, lanes, sessions, activeKey));
  }, [projects, lanes, sessions, activeKey, target]);

  // The prompt grows with what is typed, up to a cap, then scrolls.
  // biome-ignore lint/correctness/useExhaustiveDependencies: measures after each change to the text
  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [prompt]);

  const project = projects.find((item) => item.id === target.projectId);
  const lane = lanes.find((item) => item.id === target.laneId);
  const chosenModel = models?.find((item) => item.id === model);
  const focusPrompt = () => field.current?.focus();

  const submit = async () => {
    if (target.laneId === null || busy) return;
    setBusy(true);
    await create(target.laneId, {
      model: model || undefined,
      thinking: thinking || undefined,
      prompt: prompt.trim() || undefined,
    });
    setBusy(false);
    close();
  };

  if (projects.length === 0 || !project) {
    return (
      <Overlay label="New session" width={480} onClose={close}>
        <OverlayHeader onClose={close}>
          <h2 className="text-heading">New session</h2>
        </OverlayHeader>
        <div className="flex flex-col items-center gap-3 px-6 pt-4 pb-8 text-center">
          <Icon name="folder-open" size={24} className="text-ink-muted" />
          <p className="max-w-80 text-ink-muted">
            Sessions run in lanes, and lanes belong to projects. Add a git repository to begin.
          </p>
          <Button variant="primary" autoFocus onClick={() => void pickProjectFolder()}>
            Add a project
          </Button>
        </div>
      </Overlay>
    );
  }

  return (
    <Overlay label="New session" width={640} onClose={busy ? undefined : close}>
      <form
        className="flex min-h-0 flex-col"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <OverlayHeader onClose={busy ? undefined : close}>
          <Picker
            label="Project"
            icon="folder"
            value={project.name}
            groups={projectGroups(projects, target.projectId)}
            searchable={projects.length > 6}
            disabled={busy}
            returnFocus={focusPrompt}
            onSelect={(id) => {
              const projectId = Number(id);
              setTarget({ projectId, laneId: firstLane(lanes, projectId) });
            }}
          />
          <Icon name="chevron-right" size={12} className="flex-none text-ink-faint" />
          <Picker
            label="Lane"
            icon="git-branch"
            value={lane ? lane.task : "Choose a lane"}
            groups={laneGroups(lanes, target.projectId, target.laneId)}
            searchable={lanes.length > 6}
            width={420}
            disabled={busy}
            returnFocus={focusPrompt}
            onSelect={(id) => {
              if (id === NEW_LANE) {
                show({
                  type: "newLane",
                  projectId: project.id,
                  prompt: prompt.trim() || undefined,
                });
                return;
              }
              setTarget({ ...target, laneId: Number(id) });
            }}
          />
        </OverlayHeader>

        <textarea
          ref={field}
          aria-label="First prompt"
          aria-describedby="new-session-help"
          rows={3}
          placeholder={
            lane ? "What should the agent do first?" : "Pick a lane, or make one, to start in"
          }
          value={prompt}
          disabled={busy}
          // biome-ignore lint/a11y/noAutofocus: the prompt is what this panel is for
          autoFocus
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            // Enter starts the session, as it sends in the composer; ⇧↵ breaks the line.
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              event.currentTarget.form?.requestSubmit();
            }
          }}
          className="block max-h-[40vh] min-h-24 w-full resize-none bg-transparent px-4 pt-1 pb-4 text-body text-ink placeholder:text-ink-faint focus:outline-none disabled:text-ink-muted"
        />
        <span id="new-session-help" className="sr-only">
          Enter starts the session. Shift+Enter adds a line. Leave it empty to start without a
          prompt.
        </span>

        <OverlayFooter>
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
            returnFocus={focusPrompt}
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
              returnFocus={focusPrompt}
              onSelect={setThinking}
            />
          )}
          <span className="ml-auto hidden items-center gap-1.5 text-label font-normal text-ink-muted sm:inline-flex">
            <Kbd keys={["⇧", "↵"]} /> new line
          </span>
          <Button
            type="submit"
            variant="primary"
            kbd={busy ? undefined : ["↵"]}
            disabled={busy || target.laneId === null}
            className="max-sm:ml-auto"
          >
            {busy ? "Starting…" : "Start session"}
          </Button>
        </OverlayFooter>
      </form>
    </Overlay>
  );
}
