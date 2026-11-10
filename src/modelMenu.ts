import type { ModelInfo } from "./types";

export interface ModelRow {
  id: string;
  name: string;
  provider: string;
  current: boolean;
}

/**
 * The models matching `query`, in the harness's order. Name, id and provider
 * all match, so "sonnet", "anthropic" and "claude-sonnet" all find the same
 * model. An empty query matches everything.
 */
export function filterModels(
  models: ModelInfo[],
  query: string,
  currentId?: string | null,
): ModelRow[] {
  const needle = query.trim().toLowerCase();
  return models
    .filter((model) =>
      needle
        ? [model.name, model.id, model.provider].some((text) => text.toLowerCase().includes(needle))
        : true,
    )
    .map((model) => ({
      id: model.id,
      name: model.name,
      provider: model.provider,
      current: model.id === currentId,
    }));
}
