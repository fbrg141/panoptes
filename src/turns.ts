import type { Conversation } from "./conversation";
import type { Approval, Block, FileEdit, Message, ToolKind } from "./types";

export type ToolCallBlock = Extract<Block, { type: "toolCall" }>;

export interface ToolStep {
  type: "tool";
  key: string;
  call: ToolCallBlock;
  durationMs: number | null;
  /** Set while the call waits for the user to allow or reject it. */
  approval: Approval | null;
}

/** Consecutive read and search calls, shown as one row. */
export interface ToolGroup {
  type: "group";
  key: string;
  steps: ToolStep[];
}

export type Step =
  | { type: "text"; key: string; text: string }
  | { type: "thinking"; key: string; text: string; durationMs: number | null; live: boolean }
  | ToolStep
  | ToolGroup
  | { type: "note"; key: string; text: string }
  | { type: "image"; key: string; mimeType: string; data: string | null }
  | { type: "error"; key: string; text: string };

/** One prompt and everything the agent did in reply. */
export interface Turn {
  key: string;
  /** Null for agent output before the first prompt (a resumed or compacted session). */
  prompt: Message | null;
  /** What the agent did before its final answer: thinking, tools, interim text. */
  work: Step[];
  /** Text and errors after the last piece of work. */
  answer: Step[];
  /** The agent is still working on this turn. */
  live: boolean;
  stopped: boolean;
  startedAt: number | null;
  endedAt: number | null;
}

type Source = Pick<Conversation, "messages" | "thoughtFor" | "thinking" | "busy"> &
  Partial<Pick<Conversation, "approvals">>;

const QUIET: ToolKind[] = ["read", "search"];

/** Groups a conversation's messages into turns. */
export function buildTurns(conversation: Source): Turn[] {
  const { messages, thoughtFor, thinking, busy, approvals = [] } = conversation;

  const turns: Turn[] = [];
  let current: { prompt: Message | null; messages: Message[] } | null = null;
  const close = () => {
    if (current) turns.push(turn(current.prompt, current.messages));
  };
  const turn = (prompt: Message | null, replies: Message[]): Turn => {
    const steps: Step[] = [];
    for (const message of replies) {
      if (message.role === "other") {
        steps.push({ type: "note", key: message.id, text: textOf(message) });
        continue;
      }
      message.blocks.forEach((block, index) => {
        const key = `${message.id}/${index}`;
        if (!block) return;
        switch (block.type) {
          case "text":
            if (block.text.trim()) steps.push({ type: "text", key, text: block.text });
            break;
          case "thinking":
            if (block.text.trim()) {
              const live = thinking?.messageId === message.id && thinking.index === index;
              steps.push({
                type: "thinking",
                key,
                text: block.text,
                durationMs: thoughtFor[block.text.trim()] ?? null,
                live,
              });
            }
            break;
          case "toolCall":
            steps.push({
              type: "tool",
              key,
              call: block,
              durationMs:
                block.startedAt !== null && block.endedAt !== null
                  ? block.endedAt - block.startedAt
                  : null,
              approval: approvals.find((a) => a.toolCallId === block.id) ?? null,
            });
            break;
          case "image":
            steps.push({ type: "image", key, mimeType: block.mimeType, data: block.data });
            break;
        }
      });
      if (message.error)
        steps.push({ type: "error", key: `${message.id}/error`, text: message.error });
    }
    const grouped = groupQuiet(steps);
    let split = grouped.length;
    while (
      split > 0 &&
      (grouped[split - 1].type === "text" || grouped[split - 1].type === "error")
    ) {
      split--;
    }
    const all = prompt ? [prompt, ...replies] : replies;
    const times = all.map((m) => m.timestamp).filter((t): t is number => t !== null);
    return {
      key: (prompt ?? replies[0]).id,
      prompt,
      work: grouped.slice(0, split),
      answer: grouped.slice(split),
      live: false,
      stopped: replies.some((m) => m.stopped),
      startedAt: times.length ? Math.min(...times) : null,
      endedAt: times.length > 1 ? Math.max(...times) : null,
    };
  };

  for (const message of messages) {
    if (message.role === "user") {
      close();
      current = { prompt: message, messages: [] };
    } else {
      current ??= { prompt: null, messages: [] };
      current.messages.push(message);
    }
  }
  close();
  if (busy && turns.length > 0) {
    turns[turns.length - 1] = { ...turns[turns.length - 1], live: true, endedAt: null };
  }
  return turns;
}

/** Runs of two or more successful read and search calls become one group. */
function groupQuiet(steps: Step[]): Step[] {
  const out: Step[] = [];
  let run: ToolStep[] = [];
  const flush = () => {
    if (run.length > 1) out.push({ type: "group", key: run[0].key, steps: run });
    else out.push(...run);
    run = [];
  };
  for (const step of steps) {
    if (
      step.type === "tool" &&
      QUIET.includes(step.call.kind) &&
      step.call.status !== "failed" &&
      !step.approval
    ) {
      run.push(step);
    } else {
      flush();
      out.push(step);
    }
  }
  flush();
  return out;
}

const textOf = (message: Message) =>
  message.blocks.map((b) => (b?.type === "text" ? b.text : "")).join("\n");

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The tool calls among a turn's steps, groups opened up. */
export function toolSteps(steps: Step[]): ToolStep[] {
  return steps.flatMap((s) => (s.type === "tool" ? [s] : s.type === "group" ? s.steps : []));
}

/** Approvals whose tool call no message holds, to show on their own. */
export function looseApprovals(conversation: Source): Approval[] {
  const ids = new Set(
    conversation.messages.flatMap((m) =>
      m.blocks.flatMap((b) => (b?.type === "toolCall" ? [b.id] : [])),
    ),
  );
  return (conversation.approvals ?? []).filter((a) => !ids.has(a.toolCallId));
}

/** The text a tool call produced, whatever its kind. */
export function outputText(call: ToolCallBlock): string {
  return call.output?.text ?? "";
}

/** The file changes a tool call makes. */
export function editsOf(call: ToolCallBlock): FileEdit[] {
  return call.output?.type === "edits" ? call.output.edits : [];
}

/** "Read 3 files" or "Read 2 files, searched 1 pattern", for a group row. */
export function groupLabel(steps: ToolStep[]): string {
  const reads = steps.filter((s) => s.call.kind === "read").length;
  const searches = steps.length - reads;
  const parts = [
    reads > 0 ? `read ${plural(reads, "file", "files")}` : "",
    searches > 0 ? `searched ${plural(searches, "pattern", "patterns")}` : "",
  ].filter(Boolean);
  const label = parts.join(", ");
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/** What a finished turn's folded work row says, when it did enough to fold. */
export function summarizeWork(steps: Step[]): { title: string; detail: string } | null {
  const calls = toolSteps(steps);
  // A single call reads fine as its own row.
  if (calls.length < 2) return null;
  const of = (kind: ToolKind) => calls.filter((s) => s.call.kind === kind);
  const reads = of("read").length;
  const searches = of("search").length;
  const edited = [...new Set(of("edit").map((s) => basename(s.call.subject ?? s.call.name)))];
  const ran = of("execute").map((s) => s.call.subject ?? s.call.name);
  const other = of("other").length;
  const detail = [
    reads > 0 ? `read ${plural(reads, "file", "files")}` : "",
    searches > 0 ? `searched ${plural(searches, "pattern", "patterns")}` : "",
    edited.length > 0 ? `edited ${edited.slice(0, 3).join(", ")}${more(edited.length - 3)}` : "",
    ran.length > 0 ? `ran ${ran[0]}${more(ran.length - 1)}` : "",
    other > 0 ? `used ${plural(other, "other tool", "other tools")}` : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return { title: `Ran ${plural(calls.length, "tool", "tools")}`, detail };
}

const more = (n: number) => (n > 0 ? ` and ${n} more` : "");

/** A turn's answer as markdown, for copying. */
export function answerText(turn: Turn): string {
  return turn.answer
    .map((s) => (s.type === "text" ? s.text.trim() : ""))
    .filter(Boolean)
    .join("\n\n");
}

/** "8s", "1m 12s", for durations shown in rows. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${(ms / 1000).toFixed(1)}s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
