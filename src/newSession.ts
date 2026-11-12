// Model and thinking pickers, kept out of the composer so they can be tested
// without a window.
import type { PickerGroup } from "./picker";
import type { ModelInfo } from "./types";

/** The picker's first row: leave the choice to the harness. */
export const DEFAULT_CHOICE = "";

/** Models by provider, after a row that keeps the harness's default. */
export function modelGroups(models: ModelInfo[], modelId: string): PickerGroup[] {
  const groups: PickerGroup[] = [
    {
      items: [
        { id: DEFAULT_CHOICE, label: "Harness default", current: modelId === DEFAULT_CHOICE },
      ],
    },
  ];
  for (const model of models) {
    let group = groups.find((item) => item.label === model.provider);
    if (!group) {
      group = { label: model.provider, items: [] };
      groups.push(group);
    }
    group.items.push({
      id: model.id,
      label: model.name,
      detail: shortModelId(model),
      keywords: [model.provider, model.id],
      current: model.id === modelId,
    });
  }
  return groups;
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
