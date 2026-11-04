import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Answer,
  AppInfo,
  Listing,
  Message,
  OpenedSession,
  PromptMode,
  PromptOutcome,
  SessionEvent,
  SidebarSession,
} from "./types";

/** Typed wrappers for the commands in src-tauri/src/lib.rs. */
export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listSessions: () => invoke<Listing>("list_sessions"),
  openSession: (s: Pick<SidebarSession, "harness" | "key" | "locator" | "cwd">) =>
    invoke<OpenedSession>("open_session", {
      harness: s.harness,
      key: s.key,
      locator: s.locator,
      cwd: s.cwd,
    }),
  newSession: (harness: string, cwd: string) =>
    invoke<OpenedSession>("new_session", { harness, cwd }),
  history: (key: string) => invoke<Message[]>("session_history", { key }),
  sendPrompt: (key: string, text: string, mode: PromptMode) =>
    invoke<PromptOutcome>("send_prompt", { key, text, mode }),
  abort: (key: string) => invoke<void>("abort_run", { key }),
  answer: (key: string, questionId: string, answer: Answer) =>
    invoke<void>("answer_question", { key, questionId, answer }),
  closeSession: (key: string) => invoke<void>("close_session", { key }),
  setPinned: (key: string, pinned: boolean) => invoke<void>("set_pinned", { key, pinned }),
  onSessionEvent: (handler: (event: SessionEvent) => void) =>
    listen<SessionEvent>("session-event", (e) => handler(e.payload)),
};
