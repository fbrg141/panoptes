import { Button, Icon, Menu, type MenuItem } from "@panoptes/design-system";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { api } from "../api";
import {
  commandAt,
  composePrompt,
  contextLabel,
  continueList,
  insertCommand,
  insertMention,
  type MentionQuery,
  matchCommands,
  matchFiles,
  mentionAt,
  nextLevel,
  restoreUnsent,
  segments,
} from "../composer";
import { modelGroups, shortModelId } from "../newSession";
import { describeError, useStore } from "../store";
import { proseField } from "../textFields";
import type { CommandInfo, FileAttachment, QueuedPrompt, SessionSettings } from "../types";
import { Picker } from "./Picker";
import { SessionUsage } from "./SessionUsage";

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

/** Shows a failure in the window's error banner, as the store does. */
const fail = (error: unknown) => useStore.setState({ error: describeError(error) });

const isTauri = () => "__TAURI_INTERNALS__" in window && "metadata" in window.__TAURI_INTERNALS__;

declare global {
  interface Window {
    __TAURI_INTERNALS__: Record<string, unknown>;
  }
}

export function Composer({
  sessionKey,
  busy,
  sending,
}: {
  sessionKey: string;
  busy: boolean;
  sending: boolean;
}) {
  const cwd = useStore((s) => s.sessions.find((x) => x.key === sessionKey)?.cwd);
  const settings = useStore((s) => s.settings[sessionKey]);
  const queued = useStore((s) => s.conversations[sessionKey]?.queued ?? NO_QUEUE);
  const id = useId();
  const helpId = `${id}-help`;
  const mentionsId = `${id}-mentions`;
  const send = useStore((s) => s.send);
  const abort = useStore((s) => s.abort);
  const setThinkingLevel = useStore((s) => s.setThinkingLevel);

  const input = useRef<HTMLTextAreaElement>(null);
  const backdrop = useRef<HTMLDivElement>(null);
  const [text, setText] = useState(() => useStore.getState().drafts[sessionKey]?.text ?? "");
  const [attachments, setAttachments] = useState<FileAttachment[]>(
    () => useStore.getState().drafts[sessionKey]?.attachments ?? [],
  );
  const [mentioned, setMentioned] = useState<ReadonlySet<string>>(
    () => new Set(useStore.getState().drafts[sessionKey]?.mentioned ?? []),
  );
  const [mention, setMention] = useState<MentionQuery | null>(null);
  const [slash, setSlash] = useState<MentionQuery | null>(null);
  const [picked, setPicked] = useState(0);
  const [files, setFiles] = useState<{ cwd: string; list: string[] } | null>(null);
  const [commands, setCommands] = useState<{ key: string; list: CommandInfo[] } | null>(null);
  const [dragging, setDragging] = useState(false);
  const saveDraft = useStore((s) => s.saveDraft);

  // What is typed now, kept so switching sessions can store it as a draft.
  const live = useRef<{
    text: string;
    attachments: FileAttachment[];
    mentioned: ReadonlySet<string>;
  }>({
    text,
    attachments,
    mentioned,
  });
  const key = useRef(sessionKey);

  useEffect(() => {
    live.current = { text, attachments, mentioned };
  });

  // Switching sessions stores the old one's draft and loads the new one's.
  useEffect(() => {
    const previous = key.current;
    if (previous !== sessionKey) {
      // A closed session has no draft to keep; `forget` already dropped it.
      if (previous && useStore.getState().conversations[previous]) {
        saveDraft(previous, {
          text: live.current.text,
          attachments: live.current.attachments,
          mentioned: [...live.current.mentioned],
        });
      }
      key.current = sessionKey;
      const draft = useStore.getState().drafts[sessionKey];
      setText(draft?.text ?? "");
      setAttachments(draft?.attachments ?? []);
      setMentioned(new Set(draft?.mentioned ?? []));
      setMention(null);
      setSlash(null);
    }
  }, [sessionKey, saveDraft]);

  // Splitting or collapsing a pane remounts its composer even though the
  // session stays open. Treat that like a session switch, not a cleared draft.
  useEffect(
    () => () => {
      const current = key.current;
      if (current && useStore.getState().conversations[current]) {
        saveDraft(current, {
          text: live.current.text,
          attachments: live.current.attachments,
          mentioned: [...live.current.mentioned],
        });
      }
    },
    [saveDraft],
  );

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
  const commandMatches = useMemo(
    () => (slash && commands?.key === sessionKey ? matchCommands(slash.query, commands.list) : []),
    [slash, commands, sessionKey],
  );

  // Commands are discovered on the first `/`, once per open session.
  useEffect(() => {
    if (!slash || commands?.key === sessionKey) return;
    setCommands({ key: sessionKey, list: [] });
    api.listCommands(sessionKey).then((list) => {
      setCommands((current) => (current?.key === sessionKey ? { key: sessionKey, list } : current));
    }, fail);
  }, [slash, commands, sessionKey]);

  const attachPaths = async (paths: string[]) => {
    try {
      const read = await Promise.all(paths.map((path) => api.readAttachment(path)));
      setAttachments((current) => [...current, ...read]);
    } catch (e) {
      fail(e);
    }
  };

  // Inside the app, the window reports dropped files by path.
  // biome-ignore lint/correctness/useExhaustiveDependencies: subscribes again for a new session
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/webview")
      .then(async ({ getCurrentWebview }) => {
        const stop = await getCurrentWebview().onDragDropEvent(({ payload }) => {
          // Native file drops reach every webview listener; only the focused
          // pane's composer should receive one.
          if (useStore.getState().activeKey !== sessionKey) return;
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
  }, [sessionKey]);

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
    const collapsed = el.selectionStart === el.selectionEnd;
    const found = collapsed ? mentionAt(el.value, el.selectionStart) : null;
    setMention(found);
    const nextSlash = collapsed && !found ? commandAt(el.value, el.selectionStart) : null;
    setSlash(nextSlash);
    if (found?.query !== mention?.query || nextSlash?.query !== slash?.query) setPicked(0);
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

  const pickCommand = (command: CommandInfo) => {
    const el = input.current;
    if (!el || !slash) return;
    const next = insertCommand(text, el.selectionStart, command);
    setText(next.text);
    setSlash(null);
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(next.caret, next.caret);
    });
  };

  const submit = (queue: boolean) => {
    if (sending || (!text.trim() && attachments.length === 0)) return;
    const prompt = composePrompt(text, attachments);
    const sentFrom = sessionKey;
    const unsent = { text, attachments, mentioned: [...mentioned] };
    setText("");
    setAttachments([]);
    setMentioned(new Set());
    setMention(null);
    setSlash(null);
    saveDraft(sessionKey, { text: "", attachments: [], mentioned: [] });
    void send(prompt.text, { images: prompt.images, queue }, sessionKey).then((delivered) => {
      if (delivered || !sentFrom) return;
      // The prompt was not accepted: give back what was typed. If the process
      // exited, it may still have arrived, and the error says to check first.
      const here = key.current === sentFrom;
      const current = here
        ? { ...live.current, mentioned: [...live.current.mentioned] }
        : useStore.getState().drafts[sentFrom];
      const draft = restoreUnsent(unsent, current);
      if (here) {
        setText(draft.text);
        setAttachments(draft.attachments);
        setMentioned(new Set(draft.mentioned));
      } else {
        saveDraft(sentFrom, draft);
      }
    });
  };

  const cycleThinking = () => {
    const level = nextLevel(settings?.thinkingLevels ?? [], settings?.thinkingLevel ?? null);
    if (level && (settings?.thinkingLevels.length ?? 0) > 1) {
      void setThinkingLevel(level, sessionKey);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    const completionRows = mention ? matches : slash ? commandMatches : [];
    if ((mention || slash) && completionRows.length > 0) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        setPicked((i) => (i + step + completionRows.length) % completionRows.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const index = Math.min(picked, completionRows.length - 1);
        if (mention) pick(completionRows[index] as string);
        else pickCommand(completionRows[index] as CommandInfo);
        return;
      }
    }
    if ((mention || slash) && e.key === "Escape") {
      e.preventDefault();
      setMention(null);
      setSlash(null);
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
    if (e.key === "Enter") {
      // A newline that continues a list is typed for the field, so the caret
      // lands after the new marker.
      const next = continueList(text, e.currentTarget.selectionStart, e.currentTarget.selectionEnd);
      if (next) {
        e.preventDefault();
        setText(next.text);
        requestAnimationFrame(() => input.current?.setSelectionRange(next.caret, next.caret));
      }
      return;
    }
    if (e.key === "Escape" && busy) void abort(sessionKey);
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
      <span id={helpId} className="sr-only">
        Enter to send. Shift+Enter for a new line, continuing a list when the line starts one. While
        the agent runs, Enter steers it, Option+Enter queues the message for when it finishes, and
        Escape stops it. Type @ to mention a file or / to choose a harness command. Shift+Tab
        changes the thinking level.
      </span>

      {mention && matches.length > 0 && (
        <MentionList
          id={mentionsId}
          matches={matches}
          picked={picked}
          onPick={pick}
          onHover={setPicked}
        />
      )}

      {slash && commandMatches.length > 0 && (
        <CommandList
          id={mentionsId}
          matches={commandMatches}
          picked={picked}
          onPick={pickCommand}
          onHover={setPicked}
        />
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
          aria-describedby={helpId}
          aria-autocomplete="list"
          aria-controls={mention || slash ? mentionsId : undefined}
          aria-activedescendant={
            (mention || slash) && (mention ? matches : commandMatches).length > 0
              ? `${mentionsId}-${picked}`
              : undefined
          }
          enterKeyHint="send"
          {...proseField}
          rows={1}
          className="relative block max-h-60 min-h-[var(--control-lg)] w-full resize-none overflow-y-auto bg-transparent px-3 pt-2.5 pb-1 text-transparent caret-ink [scrollbar-width:none] placeholder:text-ink-faint focus:outline-none"
          placeholder={
            busy
              ? "Steer the agent, ⌥↵ to queue for after"
              : "Message the agent, @ for files, / for commands"
          }
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            trackMention(e.target);
          }}
          onSelect={(e) => trackMention(e.currentTarget)}
          onBlur={() => {
            setMention(null);
            setSlash(null);
          }}
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
        {settings && <ModelPicker settings={settings} sessionKey={sessionKey} input={input} />}
        {settings && <ThinkingPicker settings={settings} sessionKey={sessionKey} input={input} />}
        <div className="flex-1" />
        <SessionUsage sessionKey={sessionKey} />
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
            onClick={() => void abort(sessionKey)}
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

function CommandList({
  id,
  matches,
  picked,
  onPick,
  onHover,
}: {
  id: string;
  matches: CommandInfo[];
  picked: number;
  onPick: (command: CommandInfo) => void;
  onHover: (index: number) => void;
}) {
  return (
    <div
      id={id}
      role="listbox"
      aria-label="Commands"
      className="absolute bottom-full left-0 z-20 mb-2 w-full max-w-md rounded-md bg-surface-raised p-1 shadow-[var(--shadow-pop)]"
    >
      {matches.map((command, i) => (
        // biome-ignore lint/a11y/useKeyWithClickEvents: the text field drives the list from the keyboard
        <div
          key={command.name}
          id={`${id}-${i}`}
          role="option"
          aria-selected={i === picked}
          tabIndex={-1}
          className={`flex min-h-[var(--control)] cursor-default items-center gap-2 rounded-sm px-2 py-1 ${
            i === picked ? "bg-selection" : ""
          }`}
          onMouseDown={(e) => e.preventDefault()}
          onMouseMove={() => onHover(i)}
          onClick={() => onPick(command)}
        >
          <Icon name="terminal" size={14} className="flex-none text-ink-muted" />
          <span className="text-code-sm flex-none text-ink">/{command.name}</span>
          <span className="min-w-0 flex-1 truncate text-label font-normal text-ink-muted">
            {command.description ?? command.kind}
          </span>
        </div>
      ))}
    </div>
  );
}

function MentionList({
  id,
  matches,
  picked,
  onPick,
  onHover,
}: {
  id: string;
  matches: string[];
  picked: number;
  onPick: (path: string) => void;
  onHover: (index: number) => void;
}) {
  return (
    <div
      id={id}
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
            id={`${id}-${i}`}
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
  returnFocus,
  disabled,
  children,
}: {
  title: string;
  label: string;
  items: MenuItem[];
  activeId?: string;
  onSelect: (id: string) => void;
  onOpen?: () => void;
  returnFocus: () => void;
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
    returnFocus();
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
        // The scroll box is the menu itself: a scrolling wrapper would clip
        // the menu's shadow, which also draws its 1px edge.
        <div className="absolute bottom-full left-0 z-20 mb-1">
          <Menu
            label={label}
            items={items}
            className="max-h-80 overflow-y-auto"
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
function ModelPicker({
  settings,
  sessionKey,
  input,
}: {
  settings: SessionSettings;
  sessionKey: string;
  input: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const models = useStore((s) => s.models[sessionKey]);
  const preferences = useStore((s) => s.preferences);
  const loadModels = useStore((s) => s.loadModels);
  const setModel = useStore((s) => s.setModel);
  const current = settings.model;
  const groups = useMemo(
    () => modelGroups(models ?? [], current?.id ?? "", preferences).slice(1),
    [models, current, preferences],
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
      onOpen={() => void loadModels(sessionKey)}
      returnFocus={() => input.current?.focus()}
      onSelect={(id) => {
        if (id !== current?.id) void setModel(id, sessionKey);
      }}
    />
  );
}

function ThinkingPicker({
  settings,
  sessionKey,
  input,
}: {
  settings: SessionSettings;
  sessionKey: string;
  input: React.RefObject<HTMLTextAreaElement | null>;
}) {
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
        if (id !== level) void setThinkingLevel(id, sessionKey);
      }}
      returnFocus={() => input.current?.focus()}
    >
      <span>Thinking: {level ?? "off"}</span>
    </MenuButton>
  );
}
