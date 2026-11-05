// Mirrors the Rust types in crates/panoptes-harness and src-tauri/src/lib.rs.

export type Role = "user" | "assistant" | "tool" | "other";

export type Block =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: "toolCall"; callId: string; name: string; args: unknown; argsText?: string }
  | { type: "toolResult"; callId: string; name: string; output: string; isError: boolean }
  | { type: "image"; mimeType: string };

export interface Message {
  id: string;
  role: Role;
  blocks: Block[];
  timestamp: number | null;
  error: string | null;
}

export type NoticeLevel = "info" | "warning" | "error";

export type QuestionKind =
  | { type: "select"; options: string[] }
  | { type: "confirm" }
  | { type: "input"; placeholder: string | null }
  | { type: "editor"; prefill: string | null };

export interface Question {
  id: string;
  title: string;
  message: string | null;
  kind: QuestionKind;
  /** Unix ms after which the harness stops waiting and picks a default. */
  expiresAt: number | null;
}

export type Answer =
  | { type: "value"; value: string }
  | { type: "confirmed"; confirmed: boolean }
  | { type: "cancelled" };

export type HarnessEvent =
  | { type: "runStarted" }
  | { type: "messageStarted"; messageId: string; role: Role }
  | { type: "blockStarted"; messageId: string; index: number; block: Block }
  | {
      type: "blockDelta";
      messageId: string;
      index: number;
      kind: "text" | "thinking" | "toolArgs";
      delta: string;
    }
  | { type: "messageCompleted"; message: Message }
  | { type: "toolStarted"; callId: string; name: string; args: unknown }
  | { type: "toolUpdated"; callId: string; output: string }
  | { type: "toolFinished"; callId: string; output: string; isError: boolean }
  | { type: "runSettled"; aborted: boolean }
  | { type: "titleChanged"; title: string | null }
  | { type: "notice"; level: NoticeLevel; text: string }
  | { type: "question"; question: Question }
  | { type: "exited"; code: number | null; stderrTail: string }
  | { type: "raw"; payload: unknown };

export interface SessionEvent {
  key: string;
  event: HarnessEvent;
}

export interface SidebarSession {
  key: string;
  harness: string;
  id: string;
  locator: string;
  cwd: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messageCount: number;
  parentLocator: string | null;
  projectRoot: string;
  pinned: boolean;
  open: boolean;
}

export interface Listing {
  sessions: SidebarSession[];
  /** One line per harness whose sessions could not be listed. */
  errors: string[];
}

export interface SessionRef {
  harness: string;
  id: string;
  locator: string;
  cwd: string;
}

export interface OpenedSession {
  key: string;
  session: SessionRef;
  history: Message[];
}

export interface HarnessInfo {
  id: string;
  program: string;
  version: string;
  supported: boolean;
}

export interface AppInfo {
  harnesses: HarnessInfo[];
  errors: string[];
}

export type PromptMode = "start" | "steer" | "followUp";

export type PromptOutcome = "started" | "queued" | "handled";

/** An image sent inline with a prompt. */
export interface ImageAttachment {
  mimeType: string;
  /** The image bytes, base64-encoded. */
  data: string;
}

export interface ModelInfo {
  /** Opaque handle for setModel. */
  id: string;
  name: string;
  provider: string;
  contextWindow: number | null;
  /** Accepts images in prompts. */
  images: boolean;
}

export interface ContextUsage {
  /** Null while unknown, just after the harness compacted the context. */
  tokens: number | null;
  window: number;
}

export interface SessionSettings {
  model: ModelInfo | null;
  thinkingLevel: string | null;
  /** Levels the current model supports, lowest first. */
  thinkingLevels: string[];
  context: ContextUsage | null;
}

/** A file dropped onto the composer: images inline, anything else by path. */
export interface FileAttachment {
  name: string;
  path: string;
  mimeType: string | null;
  data: string | null;
}
