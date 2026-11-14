// Model, thinking and profile pickers, kept out of the overlays and composer
// so they can be tested without a window.
import type { PickerGroup } from "./picker";
import type { AgentProfile, ModelInfo, Preferences } from "./types";

/** The picker's first row: leave the choice to the harness. */
export const DEFAULT_CHOICE = "";

/** Models by provider, after a row that keeps the harness's default. */
export function modelGroups(
  models: ModelInfo[],
  modelId: string,
  preferences?: Pick<Preferences, "favoriteModels" | "hiddenModels">,
): PickerGroup[] {
  const groups: PickerGroup[] = [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "Harness default", current: modelId === DEFAULT_CHOICE },
      ],
    },
  ];
  const hidden = new Set(preferences?.hiddenModels ?? []);
  const visible = models.filter((model) => !hidden.has(model.id));
  const byId = new Map(visible.map((model) => [model.id, model]));
  const favorites = (preferences?.favoriteModels ?? [])
    .map((id) => byId.get(id))
    .filter((model): model is ModelInfo => model !== undefined);
  const item = (model: ModelInfo) => ({
    id: model.id,
    label: model.name,
    detail: shortModelId(model),
    keywords: [model.provider, model.id],
    current: model.id === modelId,
  });
  if (favorites.length > 0) {
    groups.push({ label: "Favorites", items: favorites.map(item) });
  }
  const favoriteIds = new Set(favorites.map((model) => model.id));
  for (const model of visible) {
    if (favoriteIds.has(model.id)) continue;
    let group = groups.find((item) => item.label === model.provider);
    if (!group) {
      group = { label: model.provider, items: [] };
      groups.push(group);
    }
    group.items.push(item(model));
  }
  return groups;
}

/** Reusable session setups, after a row that leaves every field unset. */
export function profileGroups(profiles: AgentProfile[], profileId: number | null): PickerGroup[] {
  return [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "No profile", current: profileId === null },
        ...profiles.map((profile) => ({
          id: String(profile.id),
          label: profile.name,
          detail: profile.modelId ?? undefined,
          current: profile.id === profileId,
        })),
      ],
    },
  ];
}

/** A model's id without its provider prefix, which its group already names. */
export function shortModelId(model: Pick<ModelInfo, "id" | "provider">): string {
  return model.id.startsWith(`${model.provider}/`)
    ? model.id.slice(model.provider.length + 1)
    : model.id;
}

export function thinkingGroups(levels: string[], level: string): PickerGroup[] {
  return [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "Model default", current: level === DEFAULT_CHOICE },
        ...levels.map((item) => ({ id: item, label: item, current: item === level })),
      ],
    },
  ];
}
