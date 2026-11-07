import { Button, Diff, Icon, LoopMark, StatusPill, Terminal } from "@panoptes/design-system";
import { type ReactNode, useState } from "react";
import { diffStat, editDiff } from "../diff";
import {
  answerText,
  editsOf,
  formatDuration,
  groupLabel,
  outputText,
  type Step,
  summarizeWork,
  type ToolStep,
  type Turn,
} from "../turns";
import type { Message, ToolKind } from "../types";
import { ApprovalCard } from "./ApprovalCard";
import { Markdown } from "./Markdown";

/** Lines of output shown before "Show all". */
const CLAMP = 12;
/** Prompt lines shown before "Show all". */
const PROMPT_CLAMP = 20;

const TOOL_ICON: Record<ToolKind, string> = {
  read: "file",
  edit: "file-diff",
  search: "search",
  execute: "terminal",
  other: "command",
};

/** One prompt and the agent's reply: its work as quiet rows, then its answer. */
export function TurnView({ turn }: { turn: Turn }) {
  const summary = turn.live ? null : summarizeWork(turn.work);
  return (
    <article className="group/turn flex flex-col gap-1">
      {turn.prompt && <Prompt message={turn.prompt} />}
      <div className="flex flex-col gap-1 pt-1">
        {summary ? (
          <Folded title={summary.title} detail={summary.detail}>
            <Steps steps={turn.work} live={false} />
          </Folded>
        ) : (
          <Steps steps={turn.work} live={turn.live} />
        )}
        <Steps steps={turn.answer} live={turn.live} />
        {turn.stopped && (
          <div className="flex items-center gap-2 text-ink-muted">
            <Icon name="square" size={14} />
            Stopped
          </div>
        )}
      </div>
      {!turn.live && <Footer turn={turn} />}
    </article>
  );
}

function Prompt({ message }: { message: Message }) {
  const text = message.blocks
    .map((b) => (b?.type === "text" ? b.text : ""))
    .filter(Boolean)
    .join("\n");
  const images = message.blocks.flatMap((b) => (b?.type === "image" ? [b] : []));
  const shown = images.filter((b) => b.data);
  const hidden = images.length - shown.length;
  const lines = text.split("\n");
  const [all, setAll] = useState(false);
  const clamped = !all && lines.length > PROMPT_CLAMP;
  return (
    <div className="max-w-[85%] self-end rounded-md border border-line bg-surface-raised px-3 py-2">
      <div className="whitespace-pre-wrap break-words">
        {clamped ? lines.slice(0, PROMPT_CLAMP).join("\n") : text}
      </div>
      {clamped && <ShowAll count={lines.length} onClick={() => setAll(true)} />}
      {shown.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {shown.map((image, i) => (
            // Images are positional within the message.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            <Thumbnail key={i} mimeType={image.mimeType} data={image.data ?? ""} />
          ))}
        </div>
      )}
      {hidden > 0 && (
        <div className="text-caption mt-1 flex items-center gap-1 text-ink-muted select-none">
          <Icon name="file" size={14} />
          {hidden === 1 ? "1 image" : `${hidden} images`}
        </div>
      )}
    </div>
  );
}

function Steps({ steps, live }: { steps: Step[]; live: boolean }) {
  return steps.map((step) => <StepView key={step.key} step={step} live={live} />);
}

function StepView({ step, live }: { step: Step; live: boolean }) {
  switch (step.type) {
    case "text":
      return (
        <div className="max-w-[85%] self-start rounded-md border border-line bg-surface-raised px-3 py-2">
          <Markdown text={step.text} />
        </div>
      );
    case "thinking":
      return <Thinking step={step} />;
    case "tool":
      return <ToolCall step={step} live={live} />;
    case "group":
      return (
        <Disclosure
          icon={step.steps.some((s) => s.call.kind === "search") ? "search" : "file"}
          title={groupLabel(step.steps)}
          subject={step.steps.map((s) => s.call.subject ?? s.call.name).join(", ")}
          right={
            live && step.steps.some((s) => s.call.status !== "done") && <Status status="running" />
          }
        >
          <div className="flex flex-col gap-1">
            {step.steps.map((s) => (
              <ToolCall key={s.key} step={s} live={live} />
            ))}
          </div>
        </Disclosure>
      );
    case "note":
      return (
        <div className="whitespace-pre-wrap border-l-2 border-line-strong py-1 pl-3 text-ink-muted">
          {step.text}
        </div>
      );
    case "image":
      return step.data ? (
        <div className="py-1">
          <Thumbnail mimeType={step.mimeType} data={step.data} />
        </div>
      ) : (
        <div className="text-caption flex items-center gap-1 text-ink-muted select-none">
          <Icon name="file" size={14} />
          Image ({step.mimeType})
        </div>
      );
    case "error":
      return (
        <div className="flex items-start gap-2 py-1 text-error">
          <Icon name="circle-alert" size={14} className="mt-[3px] flex-none" />
          <span className="whitespace-pre-wrap">{step.text}</span>
        </div>
      );
  }
}

function Thinking({ step }: { step: Extract<Step, { type: "thinking" }> }) {
  if (step.live) {
    const last = step.text.trim().split("\n").pop() ?? "";
    return (
      <div className="flex h-[var(--control)] items-center gap-2 px-1 text-ink-muted">
        <LoopMark size={16} state="thinking" tone="current" />
        <span className="flex-none text-ink">Thinking</span>
        <span className="min-w-0 truncate">{last}</span>
      </div>
    );
  }
  return (
    <Disclosure
      icon="eye"
      title={
        step.durationMs === null ? "Thought" : `Thought for ${formatDuration(step.durationMs)}`
      }
    >
      <Markdown text={step.text} className="pl-6 text-ink-muted" />
    </Disclosure>
  );
}

function ToolCall({ step, live }: { step: ToolStep; live: boolean }) {
  const { call } = step;
  const { status } = call;
  const running = status === "running" || (live && status === "pending");
  const diffs = editsOf(call).map(editDiff);
  const stat = diffs.length > 0 ? diffStat(diffs) : null;
  const row = (
    <Disclosure
      icon={TOOL_ICON[call.kind]}
      title={call.name}
      mono
      subject={call.subject ?? undefined}
      // Running calls show their output as it comes, failures stay in view,
      // and edits show their diff; the rest open on request.
      defaultOpen={running || status === "failed" || call.kind === "edit"}
      right={
        <>
          {stat && (
            <span className="text-code-sm flex gap-1">
              <span className="text-done">+{stat.added}</span>
              <span className="text-error">−{stat.removed}</span>
            </span>
          )}
          {step.durationMs !== null && <Time>{formatDuration(step.durationMs)}</Time>}
          {step.approval ? (
            <StatusPill status="waiting" />
          ) : running ? (
            <Status status="running" />
          ) : (
            status === "failed" && <Status status="failed" />
          )}
        </>
      }
    >
      <div className="pl-6">
        <ToolBody step={step} running={running} diffs={diffs} />
      </div>
    </Disclosure>
  );
  if (!step.approval) return row;
  return (
    <div className="flex flex-col gap-1">
      {row}
      <div className="pl-6">
        <ApprovalCard approval={step.approval} pill={false} />
      </div>
    </div>
  );
}

function ToolBody({
  step,
  running,
  diffs,
}: {
  step: ToolStep;
  running: boolean;
  diffs: ReturnType<typeof editDiff>[];
}) {
  const { call } = step;
  const output = outputText(call);
  if (call.kind === "edit" && diffs.length > 0) {
    return (
      <div className="flex flex-col gap-2">
        {diffs.map((lines, i) => (
          // Edits are positional within the call.
          // biome-ignore lint/suspicious/noArrayIndexKey: see above
          <Clamp key={i} lines={lines} from="head" limit={CLAMP * 3}>
            {(shown) => <Diff file={editsOf(call)[i].path} lines={shown} />}
          </Clamp>
        ))}
        {call.status === "failed" && output && <Output text={output} />}
      </div>
    );
  }
  if (call.kind === "execute") {
    const lines = output ? output.replace(/\n$/, "").split("\n") : [];
    const exitCode = call.output?.type === "shell" ? call.output.exitCode : null;
    if (exitCode !== null && exitCode !== 0) lines.push(`exit ${exitCode}`);
    return (
      <Clamp lines={lines} from="tail" limit={CLAMP}>
        {(shown) => (
          <Terminal
            live={running}
            lines={[
              { kind: "prompt", text: call.subject ?? call.name, cwd: "" },
              ...shown.map((text) => ({ kind: "out" as const, text })),
            ]}
          />
        )}
      </Clamp>
    );
  }
  const args =
    call.kind === "other"
      ? call.input
        ? JSON.stringify(call.input, null, 2)
        : (call.argsText ?? "")
      : "";
  return (
    <div className="flex flex-col gap-2">
      {args && (
        <pre className="text-code overflow-x-auto rounded-md bg-surface-sunken p-3">{args}</pre>
      )}
      {output && <Output text={output} />}
    </div>
  );
}

function Output({ text }: { text: string }) {
  const lines = text.replace(/\n$/, "").split("\n");
  return (
    <Clamp lines={lines} from="head" limit={CLAMP}>
      {(shown) => (
        <pre className="text-code overflow-x-auto rounded-md border border-line bg-surface-sunken p-3">
          {shown.join("\n")}
        </pre>
      )}
    </Clamp>
  );
}

/** Shows the first or last `limit` items, with a button for the rest. */
function Clamp<T>({
  lines,
  from,
  limit,
  children,
}: {
  lines: T[];
  from: "head" | "tail";
  limit: number;
  children: (shown: T[]) => ReactNode;
}) {
  const [all, setAll] = useState(false);
  const clamped = !all && lines.length > limit + 2;
  const shown = clamped ? (from === "head" ? lines.slice(0, limit) : lines.slice(-limit)) : lines;
  return (
    <div className="flex flex-col items-start gap-1 [&>*:not(button)]:w-full">
      {clamped && from === "tail" && <ShowAll count={lines.length} onClick={() => setAll(true)} />}
      {children(shown)}
      {clamped && from === "head" && <ShowAll count={lines.length} onClick={() => setAll(true)} />}
    </div>
  );
}

function ShowAll({ count, onClick }: { count: number; onClick: () => void }) {
  return (
    <Button variant="ghost" size="sm" onClick={onClick}>
      Show all {count} lines
    </Button>
  );
}

/** A quiet one-line row that opens to show more. */
function Disclosure({
  icon,
  title,
  mono = false,
  subject,
  right,
  defaultOpen = false,
  children,
}: {
  icon: string;
  title: string;
  mono?: boolean;
  subject?: string;
  right?: ReactNode;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  // Null until the user toggles it, so the default can follow the call's
  // status (a running call closes once it is done).
  const [toggled, setToggled] = useState<boolean | null>(null);
  const open = toggled ?? defaultOpen;
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setToggled(!open)}
        className="flex h-[var(--control)] w-full min-w-0 cursor-default items-center gap-2 rounded-sm px-1 text-left text-ink-muted select-none hover:bg-surface-raised"
      >
        <Icon name={open ? "chevron-down" : "chevron-right"} size={14} className="text-ink-faint" />
        <Icon name={icon} size={14} />
        <span className={`flex-none ${mono ? "text-code-strong text-ink" : ""}`}>{title}</span>
        <span className="text-code-sm min-w-0 flex-1 truncate">{subject}</span>
        {right}
      </button>
      {open && children}
    </div>
  );
}

/** A finished turn's work, folded into one row. */
function Folded({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: ReactNode;
}) {
  return (
    <Disclosure icon="history" title={title} subject={detail}>
      <div className="flex flex-col gap-1 border-l border-line pl-3">{children}</div>
    </Disclosure>
  );
}

function Footer({ turn }: { turn: Turn }) {
  const [copied, setCopied] = useState(false);
  const text = answerText(turn);
  const parts = [
    turn.startedAt !== null
      ? new Date(turn.startedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      : null,
    turn.startedAt !== null && turn.endedAt !== null
      ? formatDuration(turn.endedAt - turn.startedAt)
      : null,
  ].filter(Boolean);
  if (parts.length === 0 && !text) return null;
  return (
    <div className="flex h-[var(--control-sm)] items-center gap-3 opacity-0 transition-opacity duration-[var(--duration-fast)] select-none group-hover/turn:opacity-100 focus-within:opacity-100">
      <Time>{parts.join(" · ")}</Time>
      <div className="flex-1" />
      {text && (
        <Button
          variant="ghost"
          size="sm"
          icon={copied ? "check" : "copy"}
          onClick={() => {
            void navigator.clipboard.writeText(text).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            });
          }}
        >
          {copied ? "Copied" : "Copy"}
        </Button>
      )}
    </div>
  );
}

/** An image from the conversation. */
function Thumbnail({ mimeType, data }: { mimeType: string; data: string }) {
  return (
    <img
      src={`data:${mimeType};base64,${data}`}
      alt=""
      className="max-h-40 max-w-60 rounded-md border border-line bg-surface-sunken object-contain"
    />
  );
}

const Time = ({ children }: { children: ReactNode }) => (
  <span className="text-code-sm tabular-nums text-ink-muted">{children}</span>
);

const Status = ({ status }: { status: "running" | "failed" }) => <StatusPill status={status} />;
