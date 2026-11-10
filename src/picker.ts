// The searchable picker behind the composer's model menu and the new session
// form's project, lane, model and thinking menus. Pure, so it is tested
// without a window.

export interface PickerItem {
  id: string;
  label: string;
  /** A second, quieter line of identity: an id, a branch, a path. */
  detail?: string;
  /** Extra words a search should match besides the label and detail. */
  keywords?: string[];
  /** The value in use now; it is ticked and the list opens on it. */
  current?: boolean;
  disabled?: boolean;
}

export interface PickerGroup {
  /** A heading over the group; groups without one run on from the last. */
  label?: string;
  items: PickerItem[];
}

/**
 * The groups narrowed to `query`. An item stays when its label, detail or
 * keywords contain every word typed; groups left empty drop out. Order is
 * kept, so the list does not jump about as you type.
 */
export function filterPicker(groups: PickerGroup[], query: string): PickerGroup[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return groups;
  return groups
    .map((group) => ({
      ...group,
      items: group.items.filter((item) => {
        const text = [item.label, item.detail ?? "", ...(item.keywords ?? [])]
          .join(" ")
          .toLowerCase();
        return words.every((word) => text.includes(word));
      }),
    }))
    .filter((group) => group.items.length > 0);
}

/** The items a keyboard can land on, in display order. */
export function pickable(groups: PickerGroup[]): PickerItem[] {
  return groups.flatMap((group) => group.items.filter((item) => !item.disabled));
}

/** Where the highlight starts: the current item if it is listed, else the first. */
export function initialPick(groups: PickerGroup[]): number {
  return Math.max(
    0,
    pickable(groups).findIndex((item) => item.current),
  );
}
