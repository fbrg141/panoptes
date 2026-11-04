import { create } from "zustand";
import { api } from "./api";
import { applyEvent, type Conversation, emptyConversation, removeQuestion } from "./conversation";
import type { Answer, AppInfo, OpenedSession, SessionEvent, SidebarSession } from "./types";

export interface State {
  info: AppInfo | null;
  sessions: SidebarSession[];
  /** Sessions started here that the harness has not written to disk yet. */
  drafts: SidebarSession[];
  conversations: Record<string, Conversation>;
  /** Keys with a prompt on its way to the harness. */
  sending: Record<string, boolean>;
  activeKey: string | null;
  /** Problems listing sessions, one line per harness. */
  listErrors: string[];
  error: string | null;

  init: () => Promise<void>;
  refresh: () => Promise<void>;
  open: (session: SidebarSession) => Promise<void>;
  create: (cwd: string) => Promise<void>;
  send: (text: string) => Promise<void>;
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

    const adopt = (opened: OpenedSession) => {
      set({ activeKey: opened.key });
      // Keep anything the session already said (questions, notices) and
      // take the history as the authoritative message list.
      patch(opened.key, (c) => ({ ...c, messages: opened.history, exited: null }));
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

      send: async (text) => {
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
          const outcome = await backend.sendPrompt(key, text, busy ? "steer" : "start");
          if (outcome === "started") patch(key, (c) => ({ ...c, busy: true }));
          if (outcome === "handled") void reloadHistory(key);
        } catch (e) {
          fail(e);
        } finally {
          set((s) => ({ sending: { ...s.sending, [key]: false } }));
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
          return { conversations, activeKey: s.activeKey === key ? null : s.activeKey };
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
