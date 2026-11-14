import { Button, Input, Tabs } from "@panoptes/design-system";
import { useEffect, useMemo, useState } from "react";
import { DEFAULT_CHOICE, modelGroups, shortModelId, thinkingGroups } from "../newSession";
import { useStore } from "../store";
import type { AgentProfile, ModelInfo, ProfileInput } from "../types";
import { useUi } from "../ui";
import { Overlay, OverlayFooter, OverlayHeader } from "./Overlay";
import { Picker } from "./Picker";

/** Panoptes-only session presets and model-picker presentation. */
export function SettingsDialog() {
  const close = useUi((s) => s.close);
  const preferences = useStore((s) => s.preferences);
  const [tab, setTab] = useState<"profiles" | "models">("profiles");
  const [editing, setEditing] = useState<number | "new" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const deleteProfile = useStore((s) => s.deleteProfile);
  const models = useAvailableModels();
  const levels = useThinkingLevels();

  return (
    <Overlay label="Settings" width={680} onClose={close}>
      <OverlayHeader onClose={close}>
        <h2 className="text-heading">Settings</h2>
      </OverlayHeader>
      <div className="flex-none px-4 pb-2">
        <Tabs
          tabs={[
            {
              id: "profiles",
              label: "Profiles",
              icon: "agent",
              count: preferences.profiles.length,
            },
            { id: "models", label: "Models", icon: "circle-dot" },
          ]}
          value={tab}
          onChange={(id) => {
            setTab(id as "profiles" | "models");
            setEditing(null);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {tab === "profiles" ? (
          <div className="flex flex-col gap-3">
            <p className="text-label font-normal text-ink-muted">
              Profiles set up new sessions. Empty fields keep the harness default; instructions are
              appended to its system prompt. They do not change harness defaults or existing
              sessions.
            </p>
            {editing !== null ? (
              <ProfileEditor
                key={editing}
                profile={preferences.profiles.find((profile) => profile.id === editing)}
                models={models}
                levels={levels}
                onCancel={() => setEditing(null)}
                onSaved={() => setEditing(null)}
              />
            ) : (
              <>
                {preferences.profiles.map((profile) => (
                  <div
                    key={profile.id}
                    className="flex items-center gap-3 rounded-md border border-line px-3 py-2"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-body-strong">{profile.name}</div>
                      <div className="truncate text-label font-normal text-ink-muted">
                        {profileSummary(profile)}
                      </div>
                    </div>
                    <Button variant="ghost" size="sm" onClick={() => setEditing(profile.id)}>
                      Edit
                    </Button>
                    {confirmDelete === profile.id ? (
                      <>
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => {
                            void deleteProfile(profile.id);
                            setConfirmDelete(null);
                          }}
                        >
                          Confirm delete
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setConfirmDelete(null)}>
                          Cancel
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        icon="trash-2"
                        iconOnly
                        aria-label={`Delete profile ${profile.name}`}
                        title="Delete profile"
                        onClick={() => setConfirmDelete(profile.id)}
                      />
                    )}
                  </div>
                ))}
                {preferences.profiles.length === 0 && (
                  <p className="py-5 text-center text-ink-muted">
                    No profiles yet. Save a model, thinking level or instructions for reuse.
                  </p>
                )}
                <div>
                  <Button variant="secondary" icon="plus" onClick={() => setEditing("new")}>
                    New profile
                  </Button>
                </div>
              </>
            )}
          </div>
        ) : (
          <ModelPreferences />
        )}
      </div>
    </Overlay>
  );
}

function profileSummary(profile: AgentProfile): string {
  return [
    profile.modelId ?? "Harness default model",
    profile.thinkingLevel ? `Thinking: ${profile.thinkingLevel}` : null,
    profile.instructions ? "Additional instructions" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

function ProfileEditor({
  profile,
  models,
  levels,
  onCancel,
  onSaved,
}: {
  profile?: AgentProfile;
  models: ModelInfo[];
  levels: string[];
  onCancel: () => void;
  onSaved: () => void;
}) {
  const preferences = useStore((s) => s.preferences);
  const saveProfile = useStore((s) => s.saveProfile);
  const [name, setName] = useState(profile?.name ?? "");
  const [model, setModel] = useState(profile?.modelId ?? DEFAULT_CHOICE);
  const [thinking, setThinking] = useState(profile?.thinkingLevel ?? DEFAULT_CHOICE);
  const [instructions, setInstructions] = useState(profile?.instructions ?? "");
  const [busy, setBusy] = useState(false);
  const chosen = models.find((item) => item.id === model);

  const save = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    const input: ProfileInput = {
      name,
      modelId: model || null,
      thinkingLevel: thinking || null,
      instructions: instructions.trim() || null,
    };
    if (await saveProfile(profile?.id ?? null, input)) onSaved();
    else setBusy(false);
  };

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Input
        label="Profile name"
        aria-label="Profile name"
        placeholder="Careful implementation"
        value={name}
        disabled={busy}
        onChange={(event) => setName(event.target.value)}
        autoFocus
      />
      <div className="flex flex-wrap items-center gap-2">
        <Picker
          label="Model"
          icon="agent"
          value={chosen ? shortModelId(chosen) : model || "Harness default"}
          valueText={chosen?.name ?? (model || "Harness default")}
          groups={modelGroups(models, model, preferences)}
          emptyText="Open a session to load models"
          disabled={busy}
          onSelect={setModel}
        />
        <Picker
          label="Thinking"
          icon="circle-dot"
          value={thinking ? `Thinking: ${thinking}` : "Model default"}
          groups={thinkingGroups(levels, thinking)}
          searchable={false}
          width={200}
          emptyText="Open a session to load thinking levels"
          disabled={busy}
          onSelect={setThinking}
        />
      </div>
      <label className="flex flex-col gap-1.5 text-label font-normal text-ink-muted">
        Additional instructions
        <textarea
          aria-label="Additional instructions"
          rows={5}
          placeholder="Prefer small changes. Run all relevant tests."
          value={instructions}
          disabled={busy}
          onChange={(event) => setInstructions(event.target.value)}
          className="resize-y rounded-md border border-line bg-surface px-3 py-2 text-body text-ink placeholder:text-ink-faint focus:border-focus focus:outline-none"
        />
        Appended to the harness system prompt; the default prompt stays intact.
      </label>
      <OverlayFooter>
        <Button type="button" variant="ghost" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" disabled={busy || !name.trim()} className="ml-auto">
          {busy ? "Saving…" : "Save profile"}
        </Button>
      </OverlayFooter>
    </form>
  );
}

function ModelPreferences() {
  const models = useAvailableModels();
  const preferences = useStore((s) => s.preferences);
  const activeKey = useStore((s) => s.activeKey);
  const loadModels = useStore((s) => s.loadModels);
  const setFavorite = useStore((s) => s.setModelFavorite);
  const setHidden = useStore((s) => s.setModelHidden);

  useEffect(() => {
    if (activeKey && models.length === 0) void loadModels();
  }, [activeKey, loadModels, models.length]);

  const ids = useMemo(() => {
    const all = new Set(models.map((model) => model.id));
    for (const id of [...preferences.favoriteModels, ...preferences.hiddenModels]) all.add(id);
    return [...all].sort((a, b) => {
      const modelA = models.find((model) => model.id === a);
      const modelB = models.find((model) => model.id === b);
      return `${modelA?.provider ?? ""}/${modelA?.name ?? a}`.localeCompare(
        `${modelB?.provider ?? ""}/${modelB?.name ?? b}`,
      );
    });
  }, [models, preferences]);

  return (
    <div className="flex flex-col gap-2">
      <p className="text-label font-normal text-ink-muted">
        Favorites appear first in Panoptes’ pickers. Hidden models stay out of pickers but remain
        configured in the harness and usable by existing sessions.
      </p>
      {ids.map((id) => {
        const model = models.find((item) => item.id === id);
        const favorite = preferences.favoriteModels.includes(id);
        const hidden = preferences.hiddenModels.includes(id);
        return (
          <div
            key={id}
            className="flex min-h-10 items-center gap-2 rounded-md border border-line px-3 py-1"
          >
            <div className="min-w-0 flex-1">
              <div className="truncate text-body">{model?.name ?? id}</div>
              <div className="truncate text-label font-normal text-ink-muted">
                {model ? `${model.provider} · ${shortModelId(model)}` : "Saved model id"}
              </div>
            </div>
            <Button
              variant={favorite ? "secondary" : "ghost"}
              size="sm"
              aria-pressed={favorite}
              onClick={() => void setFavorite(id, !favorite)}
            >
              {favorite ? "Favorited" : "Favorite"}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void setHidden(id, !hidden)}>
              {hidden ? "Show" : "Hide"}
            </Button>
          </div>
        );
      })}
      {ids.length === 0 && (
        <p className="py-5 text-center text-ink-muted">
          {activeKey ? "Loading models…" : "Open a session to load the models Pi offers."}
        </p>
      )}
    </div>
  );
}

/** Every model list already obtained from an open harness, deduplicated by id. */
function useAvailableModels(): ModelInfo[] {
  const bySession = useStore((s) => s.models);
  return useMemo(() => {
    const models = new Map<string, ModelInfo>();
    for (const list of Object.values(bySession)) {
      for (const model of list) if (!models.has(model.id)) models.set(model.id, model);
    }
    return [...models.values()];
  }, [bySession]);
}

function useThinkingLevels(): string[] {
  const bySession = useStore((s) => s.settings);
  return useMemo(
    () =>
      Object.values(bySession).find((settings) => settings.thinkingLevels.length > 0)
        ?.thinkingLevels ?? [],
    [bySession],
  );
}
