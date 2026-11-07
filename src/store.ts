import { create } from "zustand";
import { api } from "./api";
import { applyEvent, type Conversation, emptyConversation, removeQuestion } from "./conversation";
import type {
  Answer,
  AppInfo,
  ArchiveLaneOutcome,
  ArchiveStrategy,
  ErrorKind,
  ImageAttachment,
  Lane,
  ModelInfo,
  OpenedSession,
  Project,
  PromptMode,
  SessionEvent,
  SessionSettings,
  SessionStatus,
  SidebarSession,
} from "./types";

export interface SendOptions {
  images?: ImageAttachment[];
  /** While the agent runs, deliver once it finishes instead of steering. */
  queue?: boolean;
}

export interface State {
  info: AppInfo | null;
  sessions: SidebarSession[];
  /** Registered projects, as the backend lists them. */
  projects: Project[];
  /** Durable lanes, including archived ones. */
  lanes: Lane[];
  /** Sessions started here that the harness has not written to disk yet. */
  drafts: SidebarSession[];
  conversations: Record<string, Conversation>;
  /** Each open session's status, as the backend reports it. */
  statuses: Record<string, SessionStatus>;
  /** Keys with a prompt on its way to the harness. */
  sending: Record<string, boolean>;
  /** Model, thinking level and context use, per open session. */
  settings: Record<string, SessionSettings>;
  /** Models each open session can switch to, once asked for. */
  models: Record<string, ModelInfo[]>;
  activeKey: string | null;
  /** Problems listing sessions, one line per harness. */
  listErrors: string[];
  error: string | null;

  init: () => Promise<void>;
  refresh: () => Promise<void>;
  open: (session: SidebarSession) => Promise<void>;
  /** Starts a session in `cwd`, listed under `projectId` until the backend lists it. */
  create: (cwd: string, projectId?: number | null) => Promise<void>;
  send: (text: string, options?: SendOptions) => Promise<void>;
  loadModels: () => Promise<void>;
  setModel: (modelId: string) => Promise<void>;
  setThinkingLevel: (level: string) => Promise<void>;
  abort: () => Promise<void>;
  answer: (questionId: string, answer: Answer) => Promise<void>;
  close: (key: string) => Promise<void>;
  togglePin: (session: SidebarSession) => Promise<void>;
  /** Registers the git repository `folder` is in. */
  addProject: (folder: string) => Promise<void>;
  removeProject: (id: number) => Promise<void>;
  createLane: (projectId: number, task: string, mainCheckout: boolean) => Promise<Lane | null>;
  cancelLaneCreation: (projectId: number) => Promise<void>;
  archiveLane: (id: number, strategy: ArchiveStrategy | null) => Promise<ArchiveLaneOutcome | null>;
  restoreLane: (id: number) => Promise<void>;
  dismissError: () => void;
}

/** Commands reject with a `ServiceError`; anything else is shown as it is. */
function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error && "message" in error) return String(error.message);
  return String(error);
}

export function errorKind(error: unknown): ErrorKind {
  return typeof error === "object" && error && "kind" in error
    ? (error.kind as ErrorKind)
    : "other";
}

/** The session's process is gone; opening it again starts a new one. */
const needsReopen = (error: unknown) => ["notOpen", "exited"].includes(errorKind(error));

function draftFrom(opened: OpenedSession, projectId: number | null): SidebarSession {
  const now = Date.now();
  return {
    key: opened.key,
    harness: opened.session.harness,
    id: opened.session.id,
    locator: opened.session.locator,
    cwd: opened.session.cwd,
    title: "New session",
    createdAt: now,
    updatedAt: now,
    messageCount: 0,
    parentLocator: null,
    projectRoot: opened.session.cwd,
    checkout: null,
    projectId,
    pinned: false,
    open: true,
  };
}

/** Builds the store around an API, so tests can pass a fake one. */
export function createAppStore(backend: typeof api) {
  let listening = false;

  return create<State>((set, get) => {
    const fail = (error: unknown) => set({ error: describe(error) });

    const patch = (key: string, update: (c: Conversation) => Conversation) =>
      set((s) => ({
        conversations: {
          ...s.conversations,
          [key]: update(s.conversations[key] ?? emptyConversation()),
        },
      }));

    const loadSettings = async (key: string) => {
      try {
        const settings = await backend.settings(key);
        set((s) => ({ settings: { ...s.settings, [key]: settings } }));
      } catch {
        // Settings are a nicety; the composer works without them.
      }
    };

    const adopt = (opened: OpenedSession) => {
      set({ activeKey: opened.key });
      // Keep anything the session already said (questions, notices) and
      // take the history as the authoritative message list.
      patch(opened.key, (c) => ({ ...c, messages: opened.history, exited: null }));
      void loadSettings(opened.key);
    };

    const reloadHistory = async (key: string) => {
      try {
        const messages = await backend.history(key);
        if (get().conversations[key]) patch(key, (c) => ({ ...c, messages }));
      } catch (e) {
        fail(e);
      }
    };

    const onEvent = ({ key, event }: SessionEvent) => {
      if (event.type === "statusChanged") {
        set((s) => ({ statuses: { ...s.statuses, [key]: event.status } }));
        return;
      }
      patch(key, (c) => applyEvent(c, event));
      if (event.type === "runSettled") {
        // The history is authoritative; it keeps the ids the run streamed under.
        void reloadHistory(key);
        // The run moved the context use.
        void loadSettings(key);
        void get().refresh();
      }
      if (event.type === "exited" || event.type === "titleChanged") void get().refresh();
    };

    return {
      info: null,
      sessions: [],
      projects: [],
      lanes: [],
      drafts: [],
      conversations: {},
      statuses: {},
      sending: {},
      settings: {},
      models: {},
      activeKey: null,
      listErrors: [],
      error: null,

      init: async () => {
        // React's StrictMode runs effects twice; a second listener would
        // apply every event twice.
        if (listening) return;
        listening = true;
        try {
          await backend.onSessionEvent(onEvent);
          set({ info: await backend.appInfo() });
        } catch (e) {
          fail(e);
          return;
        }
        await get().refresh();
      },

      refresh: async () => {
        try {
          const { sessions, projects, lanes, errors } = await backend.listSessions();
          const known = new Set(sessions.map((s) => s.key));
          set((s) => ({
            sessions,
            projects,
            lanes,
            listErrors: errors,
            drafts: s.drafts.filter((d) => !known.has(d.key)),
          }));
        } catch (e) {
          fail(e);
        }
      },

      open: async (session) => {
        set({ activeKey: session.key });
        try {
          adopt(await backend.openSession(session));
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      create: async (cwd, projectId = null) => {
        try {
          const opened = await backend.newSession("pi", cwd);
          set((s) => ({ drafts: [...s.drafts, draftFrom(opened, projectId)] }));
          adopt(opened);
        } catch (e) {
          fail(e);
        }
      },

      send: async (text, { images = [], queue = false } = {}) => {
        const key = get().activeKey;
        if (!key || get().sending[key]) return;
        set((s) => ({ sending: { ...s.sending, [key]: true } }));
        const reopen = async () => {
          const session = [...get().sessions, ...get().drafts].find((s) => s.key === key);
          if (!session) throw new Error("This session is no longer listed.");
          adopt(await backend.openSession(session));
        };
        const prompt = () => {
          const busy = get().conversations[key]?.busy ?? false;
          const mode: PromptMode = !busy ? "start" : queue ? "followUp" : "steer";
          return backend.sendPrompt(key, text, images, mode);
        };
        try {
          // A session whose process has gone is resumed and prompted again, once.
          // A queued prompt shows once the harness reports its queue.
          const outcome = await prompt().catch(async (e) => {
            if (!needsReopen(e)) throw e;
            await reopen();
            return prompt();
          });
          if (outcome === "started") patch(key, (c) => ({ ...c, busy: true }));
          if (outcome === "handled") void reloadHistory(key);
        } catch (e) {
          fail(e);
        } finally {
          set((s) => ({ sending: { ...s.sending, [key]: false } }));
        }
      },

      loadModels: async () => {
        const key = get().activeKey;
        if (!key) return;
        try {
          const models = await backend.listModels(key);
          set((s) => ({ models: { ...s.models, [key]: models } }));
        } catch (e) {
          fail(e);
        }
      },

      setModel: async (modelId) => {
        const key = get().activeKey;
        if (!key) return;
        try {
          const settings = await backend.setModel(key, modelId);
          set((s) => ({ settings: { ...s.settings, [key]: settings } }));
        } catch (e) {
          fail(e);
        }
      },

      setThinkingLevel: async (level) => {
        const key = get().activeKey;
        if (!key) return;
        try {
          const settings = await backend.setThinkingLevel(key, level);
          set((s) => ({ settings: { ...s.settings, [key]: settings } }));
        } catch (e) {
          fail(e);
        }
      },

      abort: async () => {
        const key = get().activeKey;
        if (key) await backend.abort(key).catch(fail);
      },

      answer: async (questionId, answer) => {
        const key = get().activeKey;
        if (!key) return;
        patch(key, (c) => removeQuestion(c, questionId));
        await backend.answer(key, questionId, answer).catch(fail);
      },

      close: async (key) => {
        await backend.closeSession(key).catch(fail);
        set((s) => {
          const { [key]: _closed, ...conversations } = s.conversations;
          const { [key]: _status, ...statuses } = s.statuses;
          const { [key]: _settings, ...settings } = s.settings;
          const { [key]: _models, ...models } = s.models;
          return {
            conversations,
            statuses,
            settings,
            models,
            activeKey: s.activeKey === key ? null : s.activeKey,
          };
        });
        await get().refresh();
      },

      togglePin: async (session) => {
        try {
          await backend.setPinned(session.key, !session.pinned);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      addProject: async (folder) => {
        try {
          await backend.addProject(folder);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      removeProject: async (id) => {
        try {
          await backend.removeProject(id);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      createLane: async (projectId, task, mainCheckout) => {
        try {
          const lane = await backend.createLane(projectId, task, mainCheckout);
          await get().refresh();
          return lane;
        } catch (e) {
          if (errorKind(e) !== "cancelled") fail(e);
          return null;
        }
      },

      cancelLaneCreation: async (projectId) => {
        await backend.cancelLaneCreation(projectId).catch(fail);
      },

      archiveLane: async (id, strategy) => {
        try {
          const outcome = await backend.archiveLane(id, strategy);
          if (outcome.type === "archived") await get().refresh();
          return outcome;
        } catch (e) {
          fail(e);
          return null;
        }
      },

      restoreLane: async (id) => {
        try {
          await backend.restoreLane(id);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      dismissError: () => set({ error: null }),
    };
  });
}

export const useStore = createAppStore(api);
