import { Button, Icon, Menu, type MenuItem } from "@panoptes/design-system";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import {
  composePrompt,
  contextLabel,
  insertMention,
  type MentionQuery,
  matchFiles,
  mentionAt,
  nextLevel,
  segments,
} from "../composer";
import { modelGroups, shortModelId } from "../newSession";
import { useStore } from "../store";
import type { FileAttachment, QueuedPrompt, SessionSettings } from "../types";
import { Picker } from "./Picker";

/** Mentions offered at once; typing narrows them. */
const MENTION_ROWS = 8;

const NO_QUEUE: QueuedPrompt[] = [];

/** Fit the text field to its content, including lines wrapped by the layout. */
function resize(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight}px`;
}

/** Reads a pasted or dropped image as an attachment the model gets inline. */
function readImage(file: File): Promise<FileAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve({
        name: file.name || "Pasted image",
        path: file.name,
        mimeType: file.type,
        data: url.slice(url.indexOf(",") + 1),
      });
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

/** Shows a failure in the window's error banner. */
const fail = (error: unknown) =>
  useStore.setState({ error: error instanceof Error ? error.message : String(error) });

const isTauri = () => "__TAURI_INTERNALS__" in window && "metadata" in window.__TAURI_INTERNALS__;

declare global {
  interface Window {
    __TAURI_INTERNALS__: Record<string, unknown>;
  }
}

export function Composer({ busy, sending }: { busy: boolean; sending: boolean }) {
  const activeKey = useStore((s) => s.activeKey);
  const cwd = useStore((s) => s.sessions.find((x) => x.key === s.activeKey)?.cwd);
  const settings = useStore((s) => (s.activeKey ? s.settings[s.activeKey] : undefined));
  const queued = useStore((s) =>
    s.activeKey ? (s.conversations[s.activeKey]?.queued ?? NO_QUEUE) : NO_QUEUE,
  );
  const send = useStore((s) => s.send);
  const abort = useStore((s) => s.abort);
  const setThinkingLevel = useStore((s) => s.setThinkingLevel);

  const input = useRef<HTMLTextAreaElement>(null);
  const backdrop = useRef<HTMLDivElement>(null);
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<FileAttachment[]>([]);
  const [mentioned, setMentioned] = useState<ReadonlySet<string>>(new Set());
  const [mention, setMention] = useState<MentionQuery | null>(null);
  const [picked, setPicked] = useState(0);
  const [files, setFiles] = useState<{ cwd: string; list: string[] } | null>(null);
  const [dragging, setDragging] = useState(false);
  const saveDraft = useStore((s) => s.saveDraft);

  // What is typed now, kept so switching sessions can store it as a draft.
  const live = useRef<{
    text: string;
    attachments: FileAttachment[];
    mentioned: ReadonlySet<string>;
  }>({
    text: "",
    attachments: [],
    mentioned: new Set(),
  });
  const key = useRef(activeKey);

  useEffect(() => {
    live.current = { text, attachments, mentioned };
  });

  // Switching sessions stores the old one's draft and loads the new one's.
  useEffect(() => {
    const previous = key.current;
    if (previous !== activeKey) {
      // A closed session has no draft to keep; `forget` already dropped it.
      if (previous && useStore.getState().conversations[previous]) {
        saveDraft(previous, {
          text: live.current.text,
          attachments: live.current.attachments,
          mentioned: [...live.current.mentioned],
        });
      }
      key.current = activeKey;
      const draft = activeKey ? useStore.getState().drafts[activeKey] : undefined;
      setText(draft?.text ?? "");
      setAttachments(draft?.attachments ?? []);
      setMentioned(new Set(draft?.mentioned ?? []));
      setMention(null);
    }
  }, [activeKey, saveDraft]);

  // Size after the controlled value is rendered, so sending also collapses it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the text value drives the measurement
  useEffect(() => {
    if (input.current) resize(input.current);
  }, [text]);

  // The workspace listing is fetched on the first `@`, once per folder.
  useEffect(() => {
    if (!mention || !cwd || files?.cwd === cwd) return;
    setFiles({ cwd, list: [] });
    api.listFiles(cwd).then((list) => setFiles({ cwd, list }), fail);
  }, [mention, cwd, files]);

  const matches = useMemo(
    () => (mention && files ? matchFiles(mention.query, files.list, MENTION_ROWS) : []),
    [mention, files],
  );

  const attachPaths = async (paths: string[]) => {
    try {
      const read = await Promise.all(paths.map((path) => api.readAttachment(path)));
      setAttachments((current) => [...current, ...read]);
    } catch (e) {
      fail(e);
    }
  };

  // Inside the app, the window reports dropped files by path.
  // biome-ignore lint/correctness/useExhaustiveDependencies: subscribe once
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/webview")
      .then(async ({ getCurrentWebview }) => {
        const stop = await getCurrentWebview().onDragDropEvent(({ payload }) => {
          setDragging(payload.type === "enter" || payload.type === "over");
          if (payload.type === "drop") void attachPaths(payload.paths);
        });
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(fail);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const pickFiles = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const chosen = await open({ multiple: true, directory: false, defaultPath: cwd });
      if (chosen) await attachPaths(Array.isArray(chosen) ? chosen : [chosen]);
    } catch (e) {
      fail(e);
    }
    input.current?.focus();
  };

  const addImages = async (list: FileList | File[]) => {
    const images = [...list].filter((file) => file.type.startsWith("image/"));
    if (images.length === 0) return false;
    try {
      const read = await Promise.all(images.map(readImage));
      setAttachments((current) => [...current, ...read]);
    } catch (e) {
      fail(e);
    }
    return true;
  };

  const trackMention = (el: HTMLTextAreaElement) => {
    const found =
      el.selectionStart === el.selectionEnd ? mentionAt(el.value, el.selectionStart) : null;
    setMention(found);
    if (found?.query !== mention?.query) setPicked(0);
  };

  const pick = (path: string) => {
    const el = input.current;
    if (!el || !mention) return;
    const next = insertMention(text, el.selectionStart, mention, path);
    setText(next.text);
    setMentioned((current) => new Set(current).add(path));
    setMention(null);
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(next.caret, next.caret);
    });
  };

  const submit = (queue: boolean) => {
    if (sending || (!text.trim() && attachments.length === 0)) return;
    const prompt = composePrompt(text, attachments);
    setText("");
    setAttachments([]);
    setMentioned(new Set());
    setMention(null);
    if (activeKey) saveDraft(activeKey, { text: "", attachments: [], mentioned: [] });
    void send(prompt.text, { images: prompt.images, queue });
  };

  const cycleThinking = () => {
    const level = nextLevel(settings?.thinkingLevels ?? [], settings?.thinkingLevel ?? null);
    if (level && (settings?.thinkingLevels.length ?? 0) > 1) void setThinkingLevel(level);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (mention && matches.length > 0) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        setPicked((i) => (i + step + matches.length) % matches.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pick(matches[Math.min(picked, matches.length - 1)]);
        return;
      }
    }
    if (mention && e.key === "Escape") {
      e.preventDefault();
      setMention(null);
      return;
    }
    if (e.key === "Tab" && e.shiftKey) {
      e.preventDefault();
      cycleThinking();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit(busy && e.altKey);
      return;
    }
    if (e.key === "Escape" && busy) void abort();
  };

  const images = settings?.model?.images ?? true;
  const empty = !text.trim() && attachments.length === 0;

  return (
    <form
      className={`relative mx-auto flex w-full max-w-3xl flex-col rounded-lg border bg-surface-raised transition-colors duration-[var(--duration-fast)] ${
        dragging ? "border-focus" : "border-line focus-within:border-line-strong"
      }`}
      onSubmit={(e) => {
        e.preventDefault();
        submit(false);
      }}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault();
      }}
      onDrop={(e) => {
        // Only reached in a plain browser; the app reports drops by path.
        if (e.dataTransfer.files.length === 0) return;
        e.preventDefault();
        void addImages(e.dataTransfer.files);
      }}
    >
      <span id="composer-help" className="sr-only">
        Enter to send. Shift+Enter for a new line. While the agent runs, Enter steers it,
        Option+Enter queues the message for when it finishes, and Escape stops it. Type @ to mention
        a file. Shift+Tab changes the thinking level.
      </span>

      {mention && matches.length > 0 && (
        <MentionList matches={matches} picked={picked} onPick={pick} onHover={setPicked} />
      )}

      {(queued.length > 0 || attachments.length > 0) && (
        <div className="flex flex-wrap gap-1 px-2 pt-2">
          {queued.map((q) => (
            <Chip
              key={q.id}
              icon="history"
              label={q.text}
              mono={false}
              title={
                q.mode === "steer"
                  ? "Steering the agent next"
                  : "Queued for when the agent finishes"
              }
            />
          ))}
          {attachments.map((file, i) => (
            <Chip
              // biome-ignore lint/suspicious/noArrayIndexKey: the same file may be attached twice
              key={`a${i}`}
              icon={file.data ? undefined : "file"}
              thumbnail={
                file.data && file.mimeType ? `data:${file.mimeType};base64,${file.data}` : undefined
              }
              label={file.name}
              title={
                file.data && !images ? `${file.path} (this model does not read images)` : file.path
              }
              muted={Boolean(file.data) && !images}
              onRemove={() => setAttachments((current) => current.filter((_, j) => j !== i))}
            />
          ))}
        </div>
      )}

      <div className="relative">
        <div
          ref={backdrop}
          aria-hidden
          className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-3 pt-2.5 pb-1 text-ink"
        >
          {segments(text, mentioned).map((segment, i) =>
            segment.mention ? (
              // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
              <mark key={i} className="rounded-xs bg-selection text-link">
                {segment.text}
              </mark>
            ) : (
              // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
              <span key={i}>{segment.text}</span>
            ),
          )}
          {/* A trailing newline needs a character after it to take up a line. */}
          {"​"}
        </div>
        <textarea
          ref={input}
          aria-label="Message"
          aria-describedby="composer-help"
          aria-autocomplete="list"
          aria-controls={mention ? "composer-mentions" : undefined}
          aria-activedescendant={
            mention && matches.length > 0 ? `composer-mention-${picked}` : undefined
          }
          enterKeyHint="send"
          rows={1}
          className="relative block max-h-60 min-h-[var(--control-lg)] w-full resize-none overflow-y-auto bg-transparent px-3 pt-2.5 pb-1 text-transparent caret-ink [scrollbar-width:none] placeholder:text-ink-faint focus:outline-none"
          placeholder={
            busy
              ? "Steer the agent, ⌥↵ to queue for after"
              : "Message the agent, @ to mention a file"
          }
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            trackMention(e.target);
          }}
          onSelect={(e) => trackMention(e.currentTarget)}
          onBlur={() => setMention(null)}
          onScroll={(e) => {
            if (backdrop.current) backdrop.current.scrollTop = e.currentTarget.scrollTop;
          }}
          onKeyDown={onKeyDown}
          onPaste={(e) => {
            if (e.clipboardData.files.length === 0) return;
            e.preventDefault();
            void addImages(e.clipboardData.files);
          }}
        />
      </div>

      <div className="flex items-center gap-1 px-2 pb-2">
        <ToolbarButton title="Attach files" label="Attach files" onClick={() => void pickFiles()}>
          <Icon name="plus" size={16} />
        </ToolbarButton>
        {settings && <ModelPicker settings={settings} />}
        {settings && <ThinkingPicker settings={settings} />}
        <div className="flex-1" />
        {settings?.context && (
          <span
            className="text-code-sm tabular-nums text-ink-muted"
            title="Context used out of the model's window"
          >
            {contextLabel(settings.context)}
          </span>
        )}
        {busy ? (
          <Button
            variant="secondary"
            icon="square"
            iconOnly
            title="Stop (Esc)"
            aria-label="Stop"
            onClick={() => void abort()}
          />
        ) : (
          <Button
            type="submit"
            variant="primary"
            icon="arrow-up"
            iconOnly
            disabled={empty || sending}
            title="Send (Enter)"
            aria-label="Send"
          />
        )}
      </div>
    </form>
  );
}

function MentionList({
  matches,
  picked,
  onPick,
  onHover,
}: {
  matches: string[];
  picked: number;
  onPick: (path: string) => void;
  onHover: (index: number) => void;
}) {
  return (
    <div
      id="composer-mentions"
      role="listbox"
      aria-label="Files"
      className="absolute bottom-full left-0 z-20 mb-2 w-full max-w-md rounded-md bg-surface-raised p-1 shadow-[var(--shadow-pop)]"
    >
      {matches.map((path, i) => {
        const slash = path.lastIndexOf("/");
        return (
          // biome-ignore lint/a11y/useKeyWithClickEvents: the text field drives the list from the keyboard
          <div
            key={path}
            id={`composer-mention-${i}`}
            role="option"
            aria-selected={i === picked}
            tabIndex={-1}
            className={`flex h-[var(--control)] cursor-default items-center gap-2 rounded-sm px-2 ${
              i === picked ? "bg-selection" : ""
            }`}
            // Keep focus in the text field so the caret survives the click.
            onMouseDown={(e) => e.preventDefault()}
            onMouseMove={() => onHover(i)}
            onClick={() => onPick(path)}
          >
            <Icon name="file" size={14} className="flex-none text-ink-muted" />
            <span className="text-code-sm flex-none text-ink">{path.slice(slash + 1)}</span>
            <span className="text-code-sm truncate text-ink-muted">{path.slice(0, slash + 1)}</span>
          </div>
        );
      })}
    </div>
  );
}

function Chip({
  icon,
  thumbnail,
  label,
  title,
  muted = false,
  mono = true,
  onRemove,
}: {
  icon?: string;
  thumbnail?: string;
  label: string;
  title: string;
  muted?: boolean;
  /** Paths and file names; prompts people wrote are set in the body face. */
  mono?: boolean;
  onRemove?: () => void;
}) {
  return (
    <span
      title={title}
      className={`flex h-[var(--control-sm)] max-w-64 items-center gap-1 rounded-sm border border-line bg-surface-sunken pl-1 ${
        onRemove ? "" : "pr-2"
      } ${muted ? "text-ink-faint" : "text-ink"}`}
    >
      {thumbnail ? (
        <img src={thumbnail} alt="" className="size-4 flex-none rounded-xs object-cover" />
      ) : (
        icon && <Icon name={icon} size={12} className="flex-none text-ink-muted" />
      )}
      <span className={`truncate ${mono ? "text-code-sm" : ""}`}>{label}</span>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove ${label}`}
          className="flex size-5 flex-none items-center justify-center rounded-xs text-ink-muted hover:bg-selection hover:text-ink"
          onClick={onRemove}
        >
          <Icon name="x" size={12} />
        </button>
      )}
    </span>
  );
}

function ToolbarButton({
  title,
  label,
  onClick,
  expanded,
  disabled = false,
  children,
}: {
  title: string;
  label?: string;
  onClick: () => void;
  expanded?: boolean;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      disabled={disabled}
      onClick={onClick}
      className="flex h-[var(--control-sm)] min-w-[var(--control-sm)] items-center justify-center gap-1 rounded-sm px-1.5 text-ink-muted transition-colors duration-[var(--duration-fast)] hover:bg-selection hover:text-ink disabled:text-ink-faint disabled:hover:bg-transparent"
    >
      {children}
    </button>
  );
}

/** A toolbar button that opens a design-system Menu above it. */
function MenuButton({
  title,
  label,
  items,
  activeId,
  onSelect,
  onOpen,
  disabled,
  children,
}: {
  title: string;
  label: string;
  items: MenuItem[];
  activeId?: string;
  onSelect: (id: string) => void;
  onOpen?: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLElement>("[role=menu]")?.focus();
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const back = () => {
    setOpen(false);
    document.querySelector<HTMLTextAreaElement>("textarea[aria-label=Message]")?.focus();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Escape closes the open menu inside
    <div
      ref={root}
      className="relative"
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          e.stopPropagation();
          back();
        }
      }}
    >
      <ToolbarButton
        title={title}
        expanded={open}
        disabled={disabled}
        onClick={() => {
          if (!open) onOpen?.();
          setOpen(!open);
        }}
      >
        {children}
        <Icon name="chevron-down" size={12} />
      </ToolbarButton>
      {open && (
        <div className="absolute bottom-full left-0 z-20 mb-1 max-h-80 overflow-y-auto rounded-md">
          <Menu
            label={label}
            items={items}
            activeId={activeId}
            onSelect={(id) => {
              onSelect(id);
              back();
            }}
          />
        </div>
      )}
    </div>
  );
}

/**
 * The model menu: the shared searchable picker, opening upward from the
 * toolbar. Typing narrows the rows by name, id or provider.
 */
function ModelPicker({ settings }: { settings: SessionSettings }) {
  const models = useStore((s) => (s.activeKey ? s.models[s.activeKey] : undefined));
  const loadModels = useStore((s) => s.loadModels);
  const setModel = useStore((s) => s.setModel);
  const current = settings.model;
  const groups = useMemo(
    () => modelGroups(models ?? [], current?.id ?? "").slice(1),
    [models, current],
  );

  return (
    <Picker
      label="Model"
      title={current ? `Model: ${current.name} (${current.provider})` : "Choose a model"}
      value={<span className="text-code-sm">{current ? shortModelId(current) : "No model"}</span>}
      valueText={current?.name ?? "No model"}
      groups={groups}
      variant="ghost"
      placement="up"
      width={352}
      emptyText="Loading models…"
      onOpen={() => void loadModels()}
      returnFocus={focusComposer}
      onSelect={(id) => {
        if (id !== current?.id) void setModel(id);
      }}
    />
  );
}

const focusComposer = () =>
  document.querySelector<HTMLTextAreaElement>("textarea[aria-label=Message]")?.focus();

function ThinkingPicker({ settings }: { settings: SessionSettings }) {
  const setThinkingLevel = useStore((s) => s.setThinkingLevel);
  const { thinkingLevels: levels, thinkingLevel: level } = settings;
  if (levels.length === 0) return null;
  const fixed = levels.length < 2;

  return (
    <MenuButton
      title={fixed ? "This model does not think" : "Thinking level (Shift+Tab to cycle)"}
      label="Thinking level"
      disabled={fixed}
      items={levels.map((l) => ({ id: l, label: l, icon: l === level ? "check" : undefined }))}
      activeId={level ?? undefined}
      onSelect={(id) => {
        if (id !== level) void setThinkingLevel(id);
      }}
    >
      <span>Thinking: {level ?? "off"}</span>
    </MenuButton>
  );
}
