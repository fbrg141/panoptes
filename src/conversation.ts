import type { Block, HarnessEvent, Message, NoticeLevel } from "./types";

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
  /** Set once the harness process has ended. */
  exited: string | null;
}

export const emptyConversation = (messages: Message[] = []): Conversation => ({
  messages,
  busy: false,
  tools: {},
  notices: [],
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
      return updateMessage(conversation, event.messageId, (m) => {
        const blocks = [...m.blocks];
        blocks[event.index] = event.block;
        return { ...m, blocks };
      });
    case "blockDelta":
      return updateMessage(conversation, event.messageId, (m) => {
        const blocks = [...m.blocks];
        blocks[event.index] = appendDelta(blocks[event.index], event.kind, event.delta);
        return { ...m, blocks };
      });
    case "messageCompleted": {
      const exists = conversation.messages.some((m) => m.id === event.message.id);
      return exists
        ? updateMessage(conversation, event.message.id, () => event.message)
        : { ...conversation, messages: [...conversation.messages, event.message] };
    }
    case "toolStarted":
      return {
        ...conversation,
        tools: { ...conversation.tools, [event.callId]: { status: "running", output: "" } },
      };
    case "toolUpdated":
      return {
        ...conversation,
        tools: { ...conversation.tools, [event.callId]: { status: "running", output: event.output } },
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
        notices: [...conversation.notices.slice(-4), { level: event.level, text: event.text }],
      };
    case "exited":
      return {
        ...conversation,
        busy: false,
        exited: event.stderrTail.trim() || "The agent process ended.",
      };
    case "titleChanged":
    case "raw":
      return conversation;
  }
}
