import type {
  Approval,
  Block,
  HarnessEvent,
  Message,
  NoticeLevel,
  PromptMode,
  Question,
  ToolCall,
} from "./types";

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
  notices: Notice[];
  /** Questions from the harness still waiting for an answer. */
  questions: Question[];
  /** Tool calls waiting for the user to allow or reject them. */
  approvals: Approval[];
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
  notices: [],
  questions: [],
  approvals: [],
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
    case "toolCallUpdated":
      return updateToolCall(conversation, event.call);
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
    case "approval":
      return { ...conversation, approvals: [...conversation.approvals, event.approval] };
    case "exited":
      return {
        ...conversation,
        busy: false,
        questions: [],
        approvals: [],
        queued: [],
        exited:
          event.stderrTail.trim() || `The agent process ended (code ${event.code ?? "none"}).`,
      };
    case "titleChanged":
    case "raw":
      return conversation;
  }
}

/** Swaps in the latest state of a tool call, keeping the streamed argument text. */
function updateToolCall(conversation: Conversation, call: ToolCall): Conversation {
  let found = false;
  const messages = conversation.messages.map((m) => {
    if (found || !m.blocks.some((b) => b?.type === "toolCall" && b.id === call.id)) return m;
    found = true;
    return {
      ...m,
      blocks: m.blocks.map((b) =>
        b?.type === "toolCall" && b.id === call.id
          ? { type: "toolCall" as const, argsText: b.argsText, ...call }
          : b,
      ),
    };
  });
  return found ? { ...conversation, messages } : conversation;
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

/** Drops an answered question or approval. */
export function removeQuestion(conversation: Conversation, questionId: string): Conversation {
  return {
    ...conversation,
    questions: conversation.questions.filter((q) => q.id !== questionId),
    approvals: conversation.approvals.filter((a) => a.id !== questionId),
  };
}
