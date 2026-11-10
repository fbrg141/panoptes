import { useStore } from "./store";
import { useUi } from "./ui";

/**
 * ⌘N is the OS folder picker, not a React dialog: opening one overlay closes
 * any other first, and dismissing the picker clears the overlay again. That
 * keeps the window's one-overlay-at-a-time rule true while the picker is up.
 */
export async function pickProjectFolder() {
  const ui = useUi.getState();
  ui.open({ type: "newProject" });
  try {
    const { pickFolder } = await import("./folder");
    const folder = await pickFolder("Add a git repository as a project");
    if (folder) await useStore.getState().addProject(folder);
  } finally {
    if (useUi.getState().overlay?.type === "newProject") useUi.getState().close();
  }
}
