import { open as openDialog } from "@tauri-apps/plugin-dialog";

/** Asks for a folder, or null when the picker is dismissed. */
export async function pickFolder(title: string): Promise<string | null> {
  const folder = await openDialog({ directory: true, title });
  return typeof folder === "string" ? folder : null;
}
