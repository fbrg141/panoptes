import { useEffect, useMemo, useRef, useState } from "react";
import type { ToolRun } from "../conversation";
import { useStore } from "../store";
import type { Block, Message } from "../types";

export function ChatPane() {
  const activeKey = useStore((s) => s.activeKey);
  const conversation = useStore((s) => (s.activeKey ? s.conversations[s.activeKey] : undefined));
  const session = useStore((s) =>
    [...s.drafts, ...s.sessions].find((x) => x.key === s.activeKey),
  );
  const scroller = useRef<HTMLDivElement>(null);
  const messages = conversation?.messages;

  // Tool results render inside the call that produced them.
  const results = useMemo(() => {
    const map: Record<string, Extract<Block, { type: "toolResult" }>> = {};
    for (const message of messages ?? []) {
      for (const block of message.blocks) if (block.type === "toolResult") map[block.callId] = block;
    }
    return map;
  }, [messages]);

  useEffect(() => {
    const el = scroller.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 200) el.scrollTop = el.scrollHeight;
  }, [messages]);

  if (!activeKey) {
    return (
      <main className="flex flex-1 items-center justify-center text-sm text-neutral-500">
        Pick a session, or start a new one.
      </main>
    );
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col">
      <header className="border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
        <div className="truncate text-sm font-medium">{session?.title ?? "Session"}</div>
        <div className="truncate text-xs text-neutral-500">{session?.cwd}</div>
      </header>
      <div ref={scroller} className="flex-1 overflow-y-auto px-4 py-4">
        <div className="mx-auto flex max-w-3xl flex-col gap-4">
          {!conversation && <p className="text-sm text-neutral-500">Starting the agent…</p>}
          {messages
            ?.filter((m) => m.role !== "tool")
            .map((m) => (
              <MessageView key={m.id} message={m} tools={conversation?.tools ?? {}} results={results} />
            ))}
        </div>
      </div>
      {conversation?.notices.map((notice, i) => (
        <div
          key={i}
          className={`border-t px-4 py-1 text-xs ${
            notice.level === "error"
              ? "border-red-200 text-red-700 dark:border-red-900 dark:text-red-400"
              : notice.level === "warning"
                ? "border-amber-200 text-amber-700 dark:border-amber-900 dark:text-amber-400"
                : "border-neutral-200 text-neutral-500 dark:border-neutral-800"
          }`}
        >
          {notice.text}
        </div>
      ))}
      {conversation?.exited && (
        <div className="border-t border-neutral-200 px-4 py-1 text-xs text-neutral-500 dark:border-neutral-800">
          The agent process stopped. Sending a message restarts it.
        </div>
      )}
      <Composer busy={conversation?.busy ?? false} />
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
  results: Record<string, Extract<Block, { type: "toolResult" }>>;
}) {
  if (message.role === "user") {
    return (
      <div className="self-end whitespace-pre-wrap rounded-lg bg-neutral-100 px-3 py-2 text-sm dark:bg-neutral-800">
        {message.blocks.map((b) => (b.type === "text" ? b.text : b.type === "image" ? "[image]" : "")).join("\n")}
      </div>
    );
  }
  if (message.role === "other") {
    return (
      <div className="whitespace-pre-wrap border-l-2 border-neutral-300 pl-3 text-xs text-neutral-500 dark:border-neutral-700">
        {message.blocks.map((b) => (b.type === "text" ? b.text : "")).join("\n")}
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2 text-sm">
      {message.blocks.map((block, i) => {
        if (!block) return null;
        switch (block.type) {
          case "text":
            return (
              <div key={i} className="whitespace-pre-wrap leading-relaxed">
                {block.text}
              </div>
            );
          case "thinking":
            return block.text ? (
              <details key={i} className="text-xs text-neutral-500">
                <summary className="cursor-default">Thinking</summary>
                <div className="whitespace-pre-wrap pt-1">{block.text}</div>
              </details>
            ) : null;
          case "toolCall":
            return <ToolCallView key={i} block={block} run={tools[block.callId]} result={results[block.callId]} />;
          default:
            return null;
        }
      })}
      {message.error && <div className="text-xs text-red-600 dark:text-red-400">{message.error}</div>}
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
  result?: Extract<Block, { type: "toolResult" }>;
}) {
  const args = block.args ? JSON.stringify(block.args, null, 2) : (block.argsText ?? "");
  const output = result?.output ?? run?.output ?? "";
  const status = result ? (result.isError ? "error" : "done") : (run?.status ?? "pending");
  const summary = summarizeArgs(block.args);
  return (
    <details className="rounded border border-neutral-200 text-xs dark:border-neutral-800">
      <summary className="flex cursor-default items-center gap-2 px-2 py-1 font-mono">
        <span className={status === "error" ? "text-red-600" : status === "done" ? "text-green-600" : "text-blue-500"}>
          {status === "error" ? "✗" : status === "done" ? "✓" : "…"}
        </span>
        <span className="font-semibold">{block.name}</span>
        <span className="truncate text-neutral-500">{summary}</span>
      </summary>
      <pre className="overflow-x-auto border-t border-neutral-200 p-2 dark:border-neutral-800">{args}</pre>
      {output && (
        <pre className="max-h-80 overflow-auto border-t border-neutral-200 p-2 dark:border-neutral-800">{output}</pre>
      )}
    </details>
  );
}

function summarizeArgs(args: unknown): string {
  if (!args || typeof args !== "object") return "";
  const values = Object.values(args as Record<string, unknown>);
  const first = values.find((v) => typeof v === "string") as string | undefined;
  return first ? first.split("\n")[0].slice(0, 120) : "";
}

function Composer({ busy }: { busy: boolean }) {
  const send = useStore((s) => s.send);
  const abort = useStore((s) => s.abort);
  const [text, setText] = useState("");

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    setText("");
    void send(trimmed);
  };

  return (
    <div className="border-t border-neutral-200 p-3 dark:border-neutral-800">
      <div className="mx-auto flex max-w-3xl items-end gap-2">
        <textarea
          className="max-h-60 min-h-10 flex-1 resize-none rounded border border-neutral-300 bg-white px-3 py-2 text-sm dark:border-neutral-700 dark:bg-neutral-950"
          rows={Math.min(8, Math.max(1, text.split("\n").length))}
          placeholder={busy ? "Steer the agent (Enter)" : "Message (Enter to send, Shift+Enter for a new line)"}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
            if (e.key === "Escape" && busy) void abort();
          }}
        />
        {busy && (
          <button
            className="rounded border border-neutral-300 px-3 py-2 text-sm hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
            onClick={() => void abort()}
            title="Stop the current run (Esc)"
          >
            Stop
          </button>
        )}
      </div>
    </div>
  );
}
