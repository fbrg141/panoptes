import type { Block, HarnessEvent, Message, NoticeLevel, PromptMode, Question } from "./types";

export interface ToolRun {
  status: "running" | "done" | "error";
  output: string;
  /** Unix ms, as seen by the UI. */
  startedAt: number;
  endedAt: number | null;
}

export interface Notice {
  level: NoticeLevel;
  text: string;
}

/** A prompt the harness holds until the agent can take it. */
export interface QueuedPrompt {
  text: string;
  mode: Exclude<PromptMode, "start">;
}

export interface Conversation {
  messages: Message[];
  busy: boolean;
  tools: Record<string, ToolRun>;
  notices: Notice[];
  /** Questions from the harness still waiting for an answer. */
  questions: Question[];
  /** Set once the harness process has ended. */
  exited: string | null;
  /** Steers and follow-ups sent during a run that the agent has not read yet. */
  queued: QueuedPrompt[];
  /**
   * How long each thinking block took, in ms, keyed by its text: harnesses
   * do not report it, and the text survives the swap to stable ids.
   */
  thoughtFor: Record<string, number>;
  /** The thinking block streaming now, and when it started. */
  thinking: { messageId: string; index: number; at: number } | null;
}

/** Notices kept per conversation; older ones scroll away. */
const MAX_NOTICES = 5;

export const emptyConversation = (messages: Message[] = []): Conversation => ({
  messages,
  busy: false,
  tools: {},
  notices: [],
  questions: [],
  exited: null,
  queued: [],
  thoughtFor: {},
  thinking: null,
});

function updateMessage(
  conversation: Conversation,
  id: string,
  update: (message: Message) => Message,
): Conversation {
  return {
    ...conversation,
    messages: conversation.messages.map((m) => (m.id === id ? update(m) : m)),
  };
}

function updateBlock(
  conversation: Conversation,
  id: string,
  index: number,
  update: (block: Block | undefined) => Block,
): Conversation {
  return updateMessage(conversation, id, (m) => {
    const blocks = [...m.blocks];
    blocks[index] = update(blocks[index]);
    return { ...m, blocks };
  });
}

function appendDelta(block: Block | undefined, kind: string, delta: string): Block {
  if (kind === "toolArgs" && block?.type === "toolCall") {
    return { ...block, argsText: (block.argsText ?? "") + delta };
  }
  if (block && (block.type === "text" || block.type === "thinking")) {
    return { ...block, text: block.text + delta };
  }
  return kind === "thinking" ? { type: "thinking", text: delta } : { type: "text", text: delta };
}

/** Records how long the streaming thinking block took, once another block starts. */
function endThinking(conversation: Conversation, messageId: string): Conversation {
  const { thinking } = conversation;
  if (!thinking || thinking.messageId !== messageId) return conversation;
  const block = conversation.messages.find((m) => m.id === messageId)?.blocks[thinking.index];
  const thoughtFor =
    block?.type === "thinking" && block.text.trim()
      ? { ...conversation.thoughtFor, [block.text.trim()]: Date.now() - thinking.at }
      : conversation.thoughtFor;
  return { ...conversation, thinking: null, thoughtFor };
}

/**
 * Folds one harness event into a conversation. Pure apart from reading the
 * clock for durations, so it is easy to test.
 */
export function applyEvent(conversation: Conversation, event: HarnessEvent): Conversation {
  switch (event.type) {
    case "runStarted":
      return { ...conversation, busy: true, exited: null };
    case "messageStarted":
      return {
        ...conversation,
        messages: [
          ...conversation.messages,
          {
            id: event.messageId,
            role: event.role,
            blocks: [],
            timestamp: Date.now(),
            error: null,
            stopped: false,
          },
        ],
      };
    case "blockStarted": {
      const next = updateBlock(
        endThinking(conversation, event.messageId),
        event.messageId,
        event.index,
        () => event.block,
      );
      return event.block.type === "thinking"
        ? {
            ...next,
            thinking: { messageId: event.messageId, index: event.index, at: Date.now() },
          }
        : next;
    }
    case "blockDelta":
      return updateBlock(conversation, event.messageId, event.index, (block) =>
        appendDelta(block, event.kind, event.delta),
      );
    case "messageCompleted": {
      conversation = endThinking(conversation, event.message.id);
      const exists = conversation.messages.some((m) => m.id === event.message.id);
      const next = exists
        ? updateMessage(conversation, event.message.id, () => event.message)
        : { ...conversation, messages: [...conversation.messages, event.message] };
      return event.message.role === "user" ? delivered(next, event.message) : next;
    }
    case "toolStarted":
    case "toolUpdated":
      return {
        ...conversation,
        tools: {
          ...conversation.tools,
          [event.callId]: {
            status: "running",
            output: event.type === "toolUpdated" ? event.output : "",
            startedAt: conversation.tools[event.callId]?.startedAt ?? Date.now(),
            endedAt: null,
          },
        },
      };
    case "toolFinished":
      return {
        ...conversation,
        tools: {
          ...conversation.tools,
          [event.callId]: {
            status: event.isError ? "error" : "done",
            output: event.output,
            startedAt: conversation.tools[event.callId]?.startedAt ?? Date.now(),
            endedAt: Date.now(),
          },
        },
      };
    case "runSettled":
      // Anything still queued was delivered under a different text
      // (expanded by the harness) or dropped by an abort.
      return { ...conversation, busy: false, queued: [], thinking: null };
    case "notice":
      return {
        ...conversation,
        notices: [
          ...conversation.notices.slice(-(MAX_NOTICES - 1)),
          { level: event.level, text: event.text },
        ],
      };
    case "question":
      return { ...conversation, questions: [...conversation.questions, event.question] };
    case "exited":
      return {
        ...conversation,
        busy: false,
        questions: [],
        queued: [],
        exited:
          event.stderrTail.trim() || `The agent process ended (code ${event.code ?? "none"}).`,
      };
    case "titleChanged":
    case "raw":
      return conversation;
  }
}

/** The agent read a queued prompt: drop the first one with the same text. */
function delivered(conversation: Conversation, message: Message): Conversation {
  const text = message.blocks
    .map((b) => (b.type === "text" ? b.text : ""))
    .join("")
    .trim();
  const index = conversation.queued.findIndex((q) => q.text.trim() === text);
  if (index < 0) return conversation;
  return { ...conversation, queued: conversation.queued.filter((_, i) => i !== index) };
}

export function enqueue(conversation: Conversation, prompt: QueuedPrompt): Conversation {
  return { ...conversation, queued: [...conversation.queued, prompt] };
}

export function removeQuestion(conversation: Conversation, questionId: string): Conversation {
  return {
    ...conversation,
    questions: conversation.questions.filter((q) => q.id !== questionId),
  };
}
