import type { Block, HarnessEvent, Message, NoticeLevel, Question } from "./types";

export interface ToolRun {
  status: "running" | "done" | "error";
  output: string;
}

export interface Notice {
  level: NoticeLevel;
  text: string;
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

/** Folds one harness event into a conversation. Pure, so it is easy to test. */
export function applyEvent(conversation: Conversation, event: HarnessEvent): Conversation {
  switch (event.type) {
    case "runStarted":
      return { ...conversation, busy: true, exited: null };
    case "messageStarted":
      return {
        ...conversation,
        messages: [
          ...conversation.messages,
          { id: event.messageId, role: event.role, blocks: [], timestamp: Date.now(), error: null },
        ],
      };
    case "blockStarted":
      return updateBlock(conversation, event.messageId, event.index, () => event.block);
    case "blockDelta":
      return updateBlock(conversation, event.messageId, event.index, (block) =>
        appendDelta(block, event.kind, event.delta),
      );
    case "messageCompleted": {
      const exists = conversation.messages.some((m) => m.id === event.message.id);
      return exists
        ? updateMessage(conversation, event.message.id, () => event.message)
        : { ...conversation, messages: [...conversation.messages, event.message] };
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
          },
        },
      };
    case "toolFinished":
      return {
        ...conversation,
        tools: {
          ...conversation.tools,
          [event.callId]: { status: event.isError ? "error" : "done", output: event.output },
        },
      };
    case "runSettled":
      return { ...conversation, busy: false };
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
        exited:
          event.stderrTail.trim() || `The agent process ended (code ${event.code ?? "none"}).`,
      };
    case "titleChanged":
    case "raw":
      return conversation;
  }
}

export function removeQuestion(conversation: Conversation, questionId: string): Conversation {
  return {
    ...conversation,
    questions: conversation.questions.filter((q) => q.id !== questionId),
  };
}
