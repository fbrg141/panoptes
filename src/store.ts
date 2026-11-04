import { create } from "zustand";
import { api } from "./api";
import { applyEvent, emptyConversation, type Conversation } from "./conversation";
import type { AppInfo, OpenedSession, SessionEvent, SidebarSession } from "./types";

interface State {
  info: AppInfo | null;
  sessions: SidebarSession[];
  /** Sessions started here that Pi has not written to disk yet. */
  drafts: SidebarSession[];
  conversations: Record<string, Conversation>;
  activeKey: string | null;
  error: string | null;

  init: () => Promise<void>;
  refresh: () => Promise<void>;
  open: (session: SidebarSession) => Promise<void>;
  create: (cwd: string) => Promise<void>;
  send: (text: string) => Promise<void>;
  abort: () => Promise<void>;
  close: (key: string) => Promise<void>;
  togglePin: (session: SidebarSession) => Promise<void>;
  dismissError: () => void;
}

let initialized = false;

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export const useStore = create<State>((set, get) => {
  const fail = (error: unknown) => set({ error: message(error) });

  const adopt = (opened: OpenedSession) =>
    set((s) => ({
      activeKey: opened.key,
      conversations: {
        ...s.conversations,
        [opened.key]: { ...(s.conversations[opened.key] ?? emptyConversation()), messages: opened.history, exited: null },
      },
    }));

  const onEvent = ({ key, event }: SessionEvent) => {
    set((s) => ({
      conversations: {
        ...s.conversations,
        [key]: applyEvent(s.conversations[key] ?? emptyConversation(), event),
      },
    }));
    if (event.type === "runSettled") {
      // Swap live message ids for the stable ones in the session history.
      api
        .history(key)
        .then((messages) =>
          set((s) => ({
            conversations: { ...s.conversations, [key]: { ...s.conversations[key], messages } },
          })),
        )
        .catch(fail);
      void get().refresh();
    }
    if (event.type === "exited" || event.type === "titleChanged") void get().refresh();
  };

  return {
    info: null,
    sessions: [],
    drafts: [],
    conversations: {},
    activeKey: null,
    error: null,

    init: async () => {
      // React's StrictMode runs effects twice; a second listener would
      // apply every event twice.
      if (initialized) return;
      initialized = true;
      await api.onSessionEvent(onEvent);
      try {
        set({ info: await api.appInfo() });
        await get().refresh();
      } catch (e) {
        fail(e);
      }
    },

    refresh: async () => {
      try {
        const sessions = await api.listSessions();
        const known = new Set(sessions.map((s) => s.key));
        set((s) => ({ sessions, drafts: s.drafts.filter((d) => !known.has(d.key)) }));
      } catch (e) {
        fail(e);
      }
    },

    open: async (session) => {
      set({ activeKey: session.key });
      try {
        adopt(await api.openSession(session));
        void get().refresh();
      } catch (e) {
        fail(e);
      }
    },

    create: async (cwd) => {
      try {
        const opened = await api.newSession("pi", cwd);
        const now = Date.now();
        const draft: SidebarSession = {
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
          projectRoot: cwd,
          pinned: false,
          open: true,
        };
        set((s) => ({ drafts: [...s.drafts, draft] }));
        adopt(opened);
      } catch (e) {
        fail(e);
      }
    },

    send: async (text) => {
      const key = get().activeKey;
      if (!key) return;
      const conversation = get().conversations[key];
      if (conversation?.exited) {
        const session = [...get().sessions, ...get().drafts].find((s) => s.key === key);
        if (session) await get().open(session);
      }
      const busy = get().conversations[key]?.busy ?? false;
      set((s) => ({
        conversations: {
          ...s.conversations,
          [key]: { ...(s.conversations[key] ?? emptyConversation()), busy: true },
        },
      }));
      try {
        await api.sendPrompt(key, text, busy ? "steer" : "start");
      } catch (e) {
        if (!busy) {
          set((s) => ({
            conversations: { ...s.conversations, [key]: { ...s.conversations[key], busy: false } },
          }));
        }
        fail(e);
      }
    },

    abort: async () => {
      const key = get().activeKey;
      if (key) await api.abort(key).catch(fail);
    },

    close: async (key) => {
      await api.closeSession(key).catch(fail);
      set((s) => {
        const conversations = { ...s.conversations };
        delete conversations[key];
        return { conversations, activeKey: s.activeKey === key ? null : s.activeKey };
      });
      void get().refresh();
    },

    togglePin: async (session) => {
      try {
        await api.setPinned(session.key, !session.pinned);
        await get().refresh();
      } catch (e) {
        fail(e);
      }
    },

    dismissError: () => set({ error: null }),
  };
});
