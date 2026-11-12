import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AdoptionOutcome,
  Answer,
  AppInfo,
  ArchiveLaneOutcome,
  ArchiveStrategy,
  FileAttachment,
  ImageAttachment,
  Lane,
  Listing,
  Message,
  ModelInfo,
  OpenedSession,
  Project,
  ProjectSettings,
  PromptMode,
  PromptOutcome,
  Session,
  SessionEvent,
  SessionSettings,
} from "./types";

/** Typed wrappers for the commands in src-tauri/src/lib.rs. */
export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  setZoom: (scale: number) => invoke<void>("set_zoom", { scale }),
  listSessions: () => invoke<Listing>("list_sessions"),
  openSession: (s: Pick<Session, "harness" | "key" | "harnessRef" | "cwd">) =>
    invoke<OpenedSession>("open_session", {
      harness: s.harness,
      key: s.key,
      locator: s.harnessRef?.locator ?? "",
      cwd: s.cwd,
    }),
  newSession: (laneId: number, harness: string) =>
    invoke<OpenedSession>("new_session", { laneId, harness }),
  adoptSession: (
    s: Pick<Session, "harness" | "key" | "harnessRef" | "cwd">,
    laneId: number | null,
  ) =>
    invoke<AdoptionOutcome>("adopt_session", {
      harness: s.harness,
      key: s.key,
      locator: s.harnessRef?.locator ?? "",
      cwd: s.cwd,
      laneId,
    }),
  archiveSession: (key: string, archived: boolean) =>
    invoke<void>("archive_session", { key, archived }),
  renameSession: (key: string, title: string | null) =>
    invoke<void>("rename_session", { key, title }),
  history: (key: string) => invoke<Message[]>("session_history", { key }),
  sendPrompt: (key: string, text: string, images: ImageAttachment[], mode: PromptMode) =>
    invoke<PromptOutcome>("send_prompt", { key, text, images, mode }),
  settings: (key: string) => invoke<SessionSettings>("session_settings", { key }),
  listModels: (key: string) => invoke<ModelInfo[]>("list_models", { key }),
  setModel: (key: string, modelId: string) =>
    invoke<SessionSettings>("set_model", { key, modelId }),
  setThinkingLevel: (key: string, level: string) =>
    invoke<SessionSettings>("set_thinking_level", { key, level }),
  listFiles: (cwd: string) => invoke<string[]>("list_files", { cwd }),
  readAttachment: (path: string) => invoke<FileAttachment>("read_attachment", { path }),
  abort: (key: string) => invoke<void>("abort_run", { key }),
  answer: (key: string, questionId: string, answer: Answer) =>
    invoke<void>("answer_question", { key, questionId, answer }),
  closeSession: (key: string) => invoke<void>("close_session", { key }),
  setPinned: (key: string, pinned: boolean) => invoke<void>("set_pinned", { key, pinned }),
  addProject: (folder: string) => invoke<Project>("add_project", { folder }),
  updateProject: (id: number, settings: ProjectSettings) =>
    invoke<Project>("update_project", { id, settings }),
  removeProject: (id: number) => invoke<void>("remove_project", { id }),
  createLane: (projectId: number, task: string, mainCheckout: boolean) =>
    invoke<Lane>("create_lane", { projectId, task, mainCheckout }),
  cancelLaneCreation: (projectId: number) => invoke<void>("cancel_lane_creation", { projectId }),
  archiveLane: (id: number, strategy: ArchiveStrategy | null) =>
    invoke<ArchiveLaneOutcome>("archive_lane", { id, strategy }),
  restoreLane: (id: number) => invoke<Lane>("restore_lane", { id }),
  onSessionEvent: (handler: (event: SessionEvent) => void) =>
    listen<SessionEvent>("session-event", (e) => handler(e.payload)),
};
