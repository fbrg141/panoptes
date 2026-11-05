import { Button, Icon, LoopMark, StatusPill } from "@panoptes/design-system";
import { useEffect, useMemo, useRef, useState } from "react";
import { sessionStatus, type ToolRun } from "../conversation";
import { useStore } from "../store";
import type { Block, Message } from "../types";
import { Markdown } from "./Markdown";
import { QuestionCard } from "./QuestionCard";

/** Messages rendered at first; earlier ones load on request. */
const PAGE = 150;

type ToolResult = Extract<Block, { type: "toolResult" }>;

export function ChatPane() {
  const activeKey = useStore((s) => s.activeKey);
  const conversation = useStore((s) => (s.activeKey ? s.conversations[s.activeKey] : undefined));
  const sending = useStore((s) => (s.activeKey ? (s.sending[s.activeKey] ?? false) : false));
  const session = useStore((s) => [...s.drafts, ...s.sessions].find((x) => x.key === s.activeKey));
  const scroller = useRef<HTMLDivElement>(null);
  const [shown, setShown] = useState(PAGE);
  const messages = conversation?.messages;
  const status = sessionStatus(conversation);

  // Tool results render inside the call that produced them, not on their own.
  const { visible, results } = useMemo(() => {
    const results: Record<string, ToolResult> = {};
    const visible: Message[] = [];
    for (const message of messages ?? []) {
      for (const block of message.blocks) {
        if (block?.type === "toolResult") results[block.callId] = block;
      }
      if (message.role !== "tool") visible.push(message);
    }
    return { visible, results };
  }, [messages]);

  // A different session starts from its latest page, scrolled to the end.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs per session
  useEffect(() => {
    setShown(PAGE);
    requestAnimationFrame(() => {
      const el = scroller.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, [activeKey]);

  // Follow new output, unless the user has scrolled up to read.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs per update
  useEffect(() => {
    const el = scroller.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 200) {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages]);

  if (!activeKey) {
    return (
      <main className="flex flex-1 flex-col items-center justify-center gap-4 text-center">
        <LoopMark size={96} />
        <h1 className="text-display">Panoptes</h1>
        <p className="text-ink-muted">Pick a session, or start a new one with the + button.</p>
      </main>
    );
  }

  const hidden = Math.max(0, visible.length - shown);

  return (
    <main className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-[var(--titlebar)] items-center gap-3 border-b border-line px-4">
        {status === "running" && <LoopMark size={20} state="thinking" tone="current" />}
        <div className="min-w-0 flex-1">
          <div className="text-body-strong truncate">{session?.title ?? "Session"}</div>
        </div>
        <span className="text-code-sm truncate text-ink-muted" title={session?.cwd}>
          {session?.cwd}
        </span>
        {status !== "idle" && <StatusPill status={status} />}
      </header>
      <div ref={scroller} className="flex-1 overflow-y-auto px-4 py-6">
        <div className="mx-auto flex max-w-3xl flex-col gap-4">
          {!conversation && (
            <div className="flex items-center gap-3 text-ink-muted">
              <LoopMark size={20} state="loading" tone="current" />
              Starting the agent…
            </div>
          )}
          {hidden > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="self-center"
              onClick={() => setShown((n) => n + PAGE)}
            >
              Show {Math.min(PAGE, hidden)} earlier messages ({hidden} hidden)
            </Button>
          )}
          {visible.slice(hidden).map((m) => (
            <MessageView
              key={m.id}
              message={m}
              tools={conversation?.tools ?? {}}
              results={results}
            />
          ))}
        </div>
      </div>
      {conversation?.notices.map((notice, i) => (
        <div
          // Notices are append-only and capped, so the index is stable.
          // biome-ignore lint/suspicious/noArrayIndexKey: see above
          key={i}
          className={`flex items-center gap-2 border-t border-line px-4 py-1 ${
            notice.level === "error"
              ? "bg-error-soft text-error"
              : notice.level === "warning"
                ? "bg-waiting-soft text-waiting"
                : "text-ink-muted"
          }`}
        >
          {notice.level !== "info" && <Icon name="circle-alert" size={14} />}
          {notice.text}
        </div>
      ))}
      {conversation?.exited && (
        <div
          className="flex items-center gap-2 border-t border-line bg-error-soft px-4 py-1 text-error"
          title={conversation.exited}
        >
          <Icon name="circle-alert" size={14} />
          The agent process stopped. Send a message to restart it.
        </div>
      )}
      <div className="p-3">
        {conversation?.questions.map((q) => (
          <QuestionCard key={q.id} question={q} />
        ))}
        <Composer busy={conversation?.busy ?? false} sending={sending} />
      </div>
    </main>
  );
}

function MessageView({
  message,
  tools,
  results,
}: {
  message: Message;
  tools: Record<string, ToolRun>;
  results: Record<string, ToolResult>;
}) {
  if (message.role === "user") {
    return (
      <div className="max-w-[85%] self-end whitespace-pre-wrap rounded-md border border-line bg-surface-raised px-3 py-2">
        {message.blocks
          .map((b) => (b?.type === "text" ? b.text : b?.type === "image" ? "[image]" : ""))
          .join("\n")}
      </div>
    );
  }
  if (message.role === "other") {
    return (
      <div className="whitespace-pre-wrap border-l-2 border-line-strong pl-3 text-ink-muted">
        {message.blocks.map((b) => (b?.type === "text" ? b.text : "")).join("\n")}
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      {message.blocks.map((block, i) => {
        if (!block) return null;
        // Blocks are positional (a message's content index), so the index
        // is their identity.
        switch (block.type) {
          case "text":
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            return <Markdown key={i} text={block.text} />;
          case "thinking":
            return block.text ? (
              // biome-ignore lint/suspicious/noArrayIndexKey: see above
              <details key={i} className="text-ink-muted">
                <summary className="text-label cursor-default">Thinking</summary>
                <div className="whitespace-pre-wrap pt-1">{block.text}</div>
              </details>
            ) : null;
          case "toolCall":
            return (
              <ToolCallView
                // biome-ignore lint/suspicious/noArrayIndexKey: see above
                key={i}
                block={block}
                run={tools[block.callId]}
                result={results[block.callId]}
              />
            );
          default:
            return null;
        }
      })}
      {message.error && (
        <div className="flex items-center gap-2 text-error">
          <Icon name="circle-alert" size={14} />
          {message.error}
        </div>
      )}
    </div>
  );
}

function ToolCallView({
  block,
  run,
  result,
}: {
  block: Extract<Block, { type: "toolCall" }>;
  run?: ToolRun;
  result?: ToolResult;
}) {
  const args = block.args ? JSON.stringify(block.args, null, 2) : (block.argsText ?? "");
  const output = result?.output ?? run?.output ?? "";
  const status = result ? (result.isError ? "error" : "done") : (run?.status ?? "pending");
  return (
    <details className="overflow-hidden rounded-md border border-line bg-surface-sunken">
      <summary className="flex h-[var(--control)] cursor-default items-center gap-2 px-2">
        <ToolStatus status={status} />
        <span className="text-code-strong">{block.name}</span>
        <span className="text-code-sm truncate text-ink-muted">{summarizeArgs(block.args)}</span>
      </summary>
      <pre className="text-code overflow-x-auto border-t border-line p-3">{args}</pre>
      {output && (
        <pre className="text-code max-h-80 overflow-auto border-t border-line p-3">{output}</pre>
      )}
    </details>
  );
}

function ToolStatus({ status }: { status: "running" | "pending" | "done" | "error" }) {
  switch (status) {
    case "done":
      return <Icon name="check" size={14} className="text-done" title="Done" />;
    case "error":
      return <Icon name="circle-alert" size={14} className="text-error" title="Failed" />;
    default:
      return (
        <Icon
          name="loader-circle"
          size={14}
          className="animate-spin text-running"
          title="Running"
        />
      );
  }
}

/** The first string argument, as a one-line hint of what a tool call does. */
function summarizeArgs(args: unknown): string {
  if (!args || typeof args !== "object") return "";
  const first = Object.values(args).find((v): v is string => typeof v === "string");
  return first ? first.split("\n")[0].slice(0, 120) : "";
}

function Composer({ busy, sending }: { busy: boolean; sending: boolean }) {
  const send = useStore((s) => s.send);
  const abort = useStore((s) => s.abort);
  const input = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState("");

  // The box grows with what is typed — wrapped lines included — up to its cap.
  const resize = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  };

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    setText("");
    if (input.current) resize(input.current);
    void send(trimmed);
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      <div className="relative flex items-end">
        <textarea
          ref={input}
          aria-label="Message"
          className="max-h-60 min-h-[var(--control-lg)] w-full resize-none overflow-y-auto rounded-sm border border-line-strong bg-transparent px-3 py-[6px] pr-10 text-ink placeholder:text-ink-faint focus:border-focus"
          placeholder={busy ? "Steer the agent" : "Message the agent (Shift+Enter for a new line)"}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            resize(e.currentTarget);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
            if (e.key === "Escape" && busy) void abort();
          }}
        />
        {busy ? (
          <Button
            variant="secondary"
            icon="square"
            iconOnly
            className="absolute bottom-1 right-1"
            title="Stop (Esc)"
            aria-label="Stop"
            onClick={() => void abort()}
          />
        ) : (
          <Button
            variant="primary"
            icon="arrow-up"
            iconOnly
            className="absolute bottom-1 right-1"
            disabled={!text.trim() || sending}
            title="Send (Enter)"
            aria-label="Send"
            onClick={submit}
          />
        )}
      </div>
    </div>
  );
}
