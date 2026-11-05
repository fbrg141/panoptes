import { create } from "zustand";
import { api } from "./api";
import {
  applyEvent,
  type Conversation,
  emptyConversation,
  enqueue,
  removeQuestion,
} from "./conversation";
import type {
  Answer,
  AppInfo,
  ImageAttachment,
  ModelInfo,
  OpenedSession,
  SessionEvent,
  SessionSettings,
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
  /** Sessions started here that the harness has not written to disk yet. */
  drafts: SidebarSession[];
  conversations: Record<string, Conversation>;
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
  create: (cwd: string) => Promise<void>;
  send: (text: string, options?: SendOptions) => Promise<void>;
  loadModels: () => Promise<void>;
  setModel: (modelId: string) => Promise<void>;
  setThinkingLevel: (level: string) => Promise<void>;
  abort: () => Promise<void>;
  answer: (questionId: string, answer: Answer) => Promise<void>;
  close: (key: string) => Promise<void>;
  togglePin: (session: SidebarSession) => Promise<void>;
  dismissError: () => void;
}

const describe = (error: unknown) => (error instanceof Error ? error.message : String(error));

function draftFrom(opened: OpenedSession): SidebarSession {
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
      patch(key, (c) => applyEvent(c, event));
      if (event.type === "runSettled") {
        // Swap live message ids for the stable ones in the session history.
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
      drafts: [],
      conversations: {},
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
          const { sessions, errors } = await backend.listSessions();
          const known = new Set(sessions.map((s) => s.key));
          set((s) => ({
            sessions,
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

      create: async (cwd) => {
        try {
          const opened = await backend.newSession("pi", cwd);
          set((s) => ({ drafts: [...s.drafts, draftFrom(opened)] }));
          adopt(opened);
        } catch (e) {
          fail(e);
        }
      },

      send: async (text, { images = [], queue = false } = {}) => {
        const key = get().activeKey;
        if (!key || get().sending[key]) return;
        set((s) => ({ sending: { ...s.sending, [key]: true } }));
        try {
          if (get().conversations[key]?.exited) {
            // The process is gone; resume the session before prompting.
            const session = [...get().sessions, ...get().drafts].find((s) => s.key === key);
            if (!session) throw new Error("This session is no longer listed.");
            adopt(await backend.openSession(session));
          }
          const busy = get().conversations[key]?.busy ?? false;
          const mode = !busy ? "start" : queue ? "followUp" : "steer";
          const outcome = await backend.sendPrompt(key, text, images, mode);
          if (outcome === "started") patch(key, (c) => ({ ...c, busy: true }));
          if (outcome === "queued" && mode !== "start")
            patch(key, (c) => enqueue(c, { text, mode }));
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
          const { [key]: _settings, ...settings } = s.settings;
          const { [key]: _models, ...models } = s.models;
          return {
            conversations,
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

      dismissError: () => set({ error: null }),
    };
  });
}

export const useStore = createAppStore(api);
