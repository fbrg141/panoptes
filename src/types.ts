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
