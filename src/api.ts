import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppInfo,
  Message,
  OpenedSession,
  PromptMode,
  SessionEvent,
  SidebarSession,
} from "./types";

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listSessions: () => invoke<SidebarSession[]>("list_sessions"),
  openSession: (s: Pick<SidebarSession, "harness" | "locator" | "cwd" | "key">) =>
    invoke<OpenedSession>("open_session", s),
  newSession: (harness: string, cwd: string) =>
    invoke<OpenedSession>("new_session", { harness, cwd }),
  history: (key: string) => invoke<Message[]>("session_history", { key }),
  sendPrompt: (key: string, text: string, mode: PromptMode) =>
    invoke<void>("send_prompt", { key, text, mode }),
  abort: (key: string) => invoke<void>("abort_run", { key }),
  closeSession: (key: string) => invoke<void>("close_session", { key }),
  setPinned: (key: string, pinned: boolean) => invoke<void>("set_pinned", { key, pinned }),
  onSessionEvent: (handler: (event: SessionEvent) => void) =>
    listen<SessionEvent>("session-event", (e) => handler(e.payload)),
};
