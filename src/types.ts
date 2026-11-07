// Mirrors the Rust types in crates/panoptes-harness and src-tauri/src/lib.rs.

export type Role = "user" | "assistant" | "other";

export type Block =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | ({ type: "toolCall"; argsText?: string } & ToolCall)
  /** `data` is the image base64-encoded, when the harness keeps it. */
  | { type: "image"; mimeType: string; data: string | null };

/**
 * One tool call with its status and output. History and live events give
 * the same values; `argsText` is only the UI's copy of arguments still
 * streaming in.
 */
export interface ToolCall {
  id: string;
  name: string;
  kind: ToolKind;
  subject: string | null;
  /** The arguments as sent; null while they stream. */
  input: unknown;
  status: ToolStatus;
  /** Null until there is something to show; edits carry their changes from the start. */
  output: ToolOutput | null;
  /** Unix ms, when the harness reports them. */
  startedAt: number | null;
  endedAt: number | null;
}

/** `pending`: written by the model but not started yet. */
export type ToolStatus = "pending" | "running" | "done" | "failed";

export type ToolOutput =
  | { type: "text"; text: string }
  | { type: "edits"; edits: FileEdit[]; text: string | null }
  /** `exitCode` is null while it runs, or when it was stopped before exiting. */
  | { type: "shell"; text: string; exitCode: number | null };

/** What a tool call does, so it can be shown without knowing tool names. */
export type ToolKind = "read" | "edit" | "search" | "execute" | "other";

/** One change a tool call makes; `oldText` is null when it writes the whole file. */
export interface FileEdit {
  path: string;
  oldText: string | null;
  newText: string;
}

export interface Message {
  id: string;
  role: Role;
  blocks: Block[];
  timestamp: number | null;
  error: string | null;
  /** The user stopped the agent while this message was being written. */
  stopped: boolean;
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

/** A request to let one tool call go ahead; answered with a decision. */
export interface Approval {
  id: string;
  toolCallId: string;
  title: string;
  message: string | null;
  /** Unix ms after which the harness stops waiting and picks a default. */
  expiresAt: number | null;
}

export type Answer =
  | { type: "value"; value: string }
  | { type: "confirmed"; confirmed: boolean }
  | { type: "decision"; allow: boolean }
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
  /** Replaces the tool-call block with the same id. */
  | { type: "toolCallUpdated"; call: ToolCall }
  /** Every prompt still waiting for the agent, in delivery order. */
  | { type: "queueChanged"; queued: QueuedPrompt[] }
  | { type: "runSettled"; aborted: boolean }
  | { type: "titleChanged"; title: string | null }
  | { type: "notice"; level: NoticeLevel; text: string }
  | { type: "question"; question: Question }
  | { type: "approval"; approval: Approval }
  | { type: "exited"; code: number | null; stderrTail: string }
  | { type: "raw"; payload: unknown };

/** Where an open session stands; worked out by the backend. */
export type SessionStatus = "idle" | "running" | "waiting" | "failed" | "done";

/** A harness event, or a status change the service adds. */
export type SessionUpdate = HarnessEvent | { type: "statusChanged"; status: SessionStatus };

export interface SessionEvent {
  key: string;
  event: SessionUpdate;
}

/** What went wrong, so the UI can pick a recovery without reading the text. */
export type ErrorKind =
  | "notOpen"
  | "unavailable"
  | "rejected"
  | "exited"
  | "timedOut"
  | "cancelled"
  | "other";

/** How every failed command rejects (`ServiceError` in src-tauri/src/service.rs). */
export interface ServiceError {
  kind: ErrorKind;
  message: string;
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
  /** Main checkout of the git repository the session runs in, shared by its worktrees; null outside git. */
  checkout: string | null;
  /** The registered project whose repository (or one of its worktrees) the session runs in. */
  projectId: number | null;
  pinned: boolean;
  open: boolean;
}

/** What can be changed about a project (`ProjectSettings` in src-tauri/src/store.rs). */
export interface ProjectSettings {
  name: string;
  /** What new worktrees branch from. */
  baseBranch: string;
  /** Untracked files copied from the main checkout into new worktrees. */
  copyFiles: string[];
  /** Run in a new worktree once it is made. */
  setup: string | null;
}

/** A registered git repository (`Project` in src-tauri/src/store.rs). */
export interface Project extends ProjectSettings {
  id: number;
  /** Path of the main checkout. */
  repo: string;
}

export type LaneState = "active" | "archived";

export interface LaneWorktree {
  projectId: number;
  path: string;
  /** Explicit opt-in: the project's own checkout, never removed by archiving. */
  mainCheckout: boolean;
  checkpointRef: string | null;
}

export interface WriteLock {
  holder: string | null;
  queue: string[];
}

/** Durable work owned by Panoptes; it exists with or without a session. */
export interface Lane {
  id: number;
  projectId: number;
  name: string;
  task: string;
  branch: string;
  state: LaneState;
  worktrees: LaneWorktree[];
  writeLock: WriteLock;
}

export type ArchiveStrategy = "commit" | "stash" | "checkpoint";
export type ArchiveLaneOutcome = { type: "archived" } | { type: "dirty"; lane: Lane };

export interface Listing {
  sessions: SidebarSession[];
  /** Every registered project, by name. */
  projects: Project[];
  /** Every lane, including archived ones. */
  lanes: Lane[];
  /** One line per harness or store collection that could not be read. */
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

/** A prompt the harness holds until the agent can take it. */
export interface QueuedPrompt {
  /** Stays the same while the prompt waits. */
  id: string;
  /** As the agent will read it (expanded, for a template). */
  text: string;
  mode: Exclude<PromptMode, "start">;
}

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
