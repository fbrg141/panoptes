import { create } from "zustand";
import { api } from "./api";
import { applyEvent, type Conversation, emptyConversation, removeQuestion } from "./conversation";
import type {
  AdoptionOutcome,
  Answer,
  AppInfo,
  ArchiveLaneOutcome,
  ArchiveStrategy,
  Draft,
  ErrorKind,
  ImageAttachment,
  Lane,
  LaneNavigation,
  ModelInfo,
  OpenedSession,
  Project,
  PromptMode,
  Session,
  SessionEvent,
  SessionSettings,
  SessionStatus,
} from "./types";

export interface SendOptions {
  images?: ImageAttachment[];
  /** While the agent runs, deliver once it finishes instead of steering. */
  queue?: boolean;
}

export interface State {
  info: AppInfo | null;
  sessions: Session[];
  /** Registered projects, as the backend lists them. */
  projects: Project[];
  /** Durable lanes, including archived ones. */
  lanes: Lane[];
  /** Backend-owned session membership for each lane. */
  navigation: LaneNavigation[];
  conversations: Record<string, Conversation>;
  /** Each open session's status, as the backend reports it. */
  statuses: Record<string, SessionStatus>;
  /** Keys with a prompt on its way to the harness. */
  sending: Record<string, boolean>;
  /** Model, thinking level and context use, per open session. */
  settings: Record<string, SessionSettings>;
  /** Models each open session can switch to, once asked for. */
  models: Record<string, ModelInfo[]>;
  /** Composer drafts kept for sessions the user switched away from. */
  drafts: Record<string, Draft>;
  activeKey: string | null;
  /** Problems listing sessions, one line per harness. */
  listErrors: string[];
  error: string | null;

  init: () => Promise<void>;
  refresh: () => Promise<void>;
  open: (session: Session) => Promise<void>;
  /** Starts a new session in a lane's worktree, configured and prompted. */
  create: (
    laneId: number,
    options?: { model?: string; thinking?: string; prompt?: string },
  ) => Promise<void>;
  /** Adopts a discovered conversation into a matching or explicitly chosen lane. */
  adopt: (session: Session, laneId?: number | null) => Promise<AdoptionOutcome | null>;
  /** Hides a lane session (closing it first) or brings it back; its lane is untouched. */
  setArchived: (key: string, archived: boolean) => Promise<void>;
  /** Renames a lane session; an empty title falls back to the harness's. */
  rename: (key: string, title: string) => Promise<void>;
  /** Resolves to whether the harness took the prompt, so a caller can keep its draft. */
  send: (text: string, options?: SendOptions) => Promise<boolean>;
  /** Keeps a composer draft under its session, or drops it once it is empty. */
  saveDraft: (key: string, draft: Draft) => void;
  loadModels: () => Promise<void>;
  setModel: (modelId: string) => Promise<void>;
  setThinkingLevel: (level: string) => Promise<void>;
  abort: () => Promise<void>;
  answer: (questionId: string, answer: Answer) => Promise<void>;
  close: (key: string) => Promise<void>;
  /** Registers the git repository `folder` is in. */
  addProject: (folder: string) => Promise<void>;
  removeProject: (id: number) => Promise<void>;
  createLane: (projectId: number, task: string, mainCheckout: boolean) => Promise<Lane | null>;
  createFromPrompt: (projectId: number, task: string, mainCheckout: boolean) => Promise<boolean>;
  cancelLaneCreation: (projectId: number) => Promise<void>;
  archiveLane: (id: number, strategy: ArchiveStrategy | null) => Promise<ArchiveLaneOutcome | null>;
  restoreLane: (id: number) => Promise<void>;
  /** Permanently deletes a lane after explicit confirmation in the UI. */
  deleteLane: (id: number) => Promise<boolean>;
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

    // What each session's messages were last decided by: a run starting or
    // a history request. A history reply is only applied if nothing newer
    // happened while it was on its way.
    const epochs = new Map<string, number>();
    const bump = (key: string) => {
      const next = (epochs.get(key) ?? 0) + 1;
      epochs.set(key, next);
      return next;
    };

    /** Whether a history fetched at `epoch` may still replace the messages. */
    const historyApplies = (key: string, epoch: number, c: Conversation) =>
      // During a run the events build the transcript, and the fetched
      // history does not hold the message that is streaming.
      epochs.get(key) === epoch && !(c.busy && c.messages.length > 0);

    const adopt = (opened: OpenedSession, epoch = bump(opened.key)) => {
      set({ activeKey: opened.key });
      // Keep anything the session already said (questions, notices) and
      // take the history as the authoritative message list.
      patch(opened.key, (c) => ({
        ...c,
        messages: historyApplies(opened.key, epoch, c) ? opened.history : c.messages,
        exited: null,
      }));
      void loadSettings(opened.key);
    };

    /** Drops everything kept about a session that is no longer open. */
    const forget = (key: string) =>
      set((s) => {
        const { [key]: _closed, ...conversations } = s.conversations;
        const { [key]: _status, ...statuses } = s.statuses;
        const { [key]: _settings, ...settings } = s.settings;
        const { [key]: _models, ...models } = s.models;
        const { [key]: _draft, ...drafts } = s.drafts;
        epochs.delete(key);
        return {
          conversations,
          statuses,
          settings,
          models,
          drafts,
          activeKey: s.activeKey === key ? null : s.activeKey,
        };
      });

    const reloadHistory = async (key: string) => {
      const epoch = bump(key);
      try {
        const messages = await backend.history(key);
        const current = get().conversations[key];
        if (current && historyApplies(key, epoch, current)) patch(key, (c) => ({ ...c, messages }));
      } catch (e) {
        fail(e);
      }
    };

    const onEvent = ({ key, event }: SessionEvent) => {
      if (event.type === "statusChanged") {
        set((s) => ({ statuses: { ...s.statuses, [key]: event.status } }));
        return;
      }
      if (event.type === "laneWritesChanged") {
        // A session was first seen editing another lane: hold that lane back now.
        void get().refresh();
        return;
      }
      if (event.type === "runStarted") bump(key);
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
      navigation: [],
      conversations: {},
      statuses: {},
      sending: {},
      settings: {},
      models: {},
      drafts: {},
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
          const { sessions, projects, lanes, navigation, errors } = await backend.listSessions();
          set({ sessions, projects, lanes, navigation, listErrors: errors });
        } catch (e) {
          fail(e);
        }
      },

      open: async (session) => {
        set({ activeKey: session.key });
        const epoch = bump(session.key);
        try {
          const opened = await backend.openSession(session);
          // An adopted conversation answers with its lane key, which has
          // its own history of events.
          adopt(opened, opened.key === session.key ? epoch : undefined);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      create: async (laneId, options) => {
        try {
          // The session is recorded before it starts, so it is listed at once.
          const opened = await backend.newSession(laneId, "pi");
          adopt(opened);
          await get().refresh();
          // Everything the form chose is applied to the new session by its own
          // key, so it survives the user opening another session meanwhile.
          const key = opened.key;
          const keep = async (settings: SessionSettings) =>
            set((s) => ({ settings: { ...s.settings, [key]: settings } }));
          if (options?.model) {
            try {
              keep(await backend.setModel(key, options.model));
            } catch (e) {
              fail(e);
            }
          }
          if (options?.thinking) {
            try {
              keep(await backend.setThinkingLevel(key, options.thinking));
            } catch (e) {
              fail(e);
            }
          }
          if (options?.prompt) {
            try {
              const outcome = await backend.sendPrompt(key, options.prompt, [], "start");
              if (outcome === "started") patch(key, (c) => ({ ...c, busy: true }));
              if (outcome === "handled") void reloadHistory(key);
            } catch (e) {
              fail(e);
            }
          }
        } catch (e) {
          fail(e);
        }
      },

      adopt: async (session, laneId = null) => {
        try {
          const outcome = await backend.adoptSession(session, laneId);
          if (outcome.type === "adopted") await get().refresh();
          return outcome;
        } catch (e) {
          fail(e);
          return null;
        }
      },

      setArchived: async (key, archived) => {
        try {
          await backend.archiveSession(key, archived);
          if (archived) forget(key);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      rename: async (key, title) => {
        const trimmed = title.trim();
        try {
          await backend.renameSession(key, trimmed === "" ? null : trimmed);
          await get().refresh();
        } catch (e) {
          fail(e);
        }
      },

      send: async (text, { images = [], queue = false } = {}) => {
        const key = get().activeKey;
        if (!key || get().sending[key]) return false;
        set((s) => ({ sending: { ...s.sending, [key]: true } }));
        const reopen = async () => {
          const session = get().sessions.find((s) => s.key === key);
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
          return true;
        } catch (e) {
          fail(e);
          return false;
        } finally {
          set((s) => ({ sending: { ...s.sending, [key]: false } }));
        }
      },

      saveDraft: (key, draft) =>
        set((s) => {
          if (!draft.text.trim() && draft.attachments.length === 0) {
            const { [key]: _dropped, ...drafts } = s.drafts;
            return { drafts };
          }
          return { drafts: { ...s.drafts, [key]: draft } };
        }),

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
        const before = get().conversations[key];
        const question = before?.questions.find((q) => q.id === questionId);
        const approval = before?.approvals.find((a) => a.id === questionId);
        patch(key, (c) => removeQuestion(c, questionId));
        try {
          await backend.answer(key, questionId, answer);
        } catch (e) {
          fail(e);
          // The harness is still waiting, so the card comes back. A process
          // that has gone asks nothing any more.
          if (needsReopen(e) || !get().conversations[key]) return;
          patch(key, (c) => ({
            ...c,
            questions:
              question && !c.questions.some((q) => q.id === questionId)
                ? [...c.questions, question]
                : c.questions,
            approvals:
              approval && !c.approvals.some((a) => a.id === questionId)
                ? [...c.approvals, approval]
                : c.approvals,
          }));
        }
      },

      close: async (key) => {
        await backend.closeSession(key).catch(fail);
        forget(key);
        await get().refresh();
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

      createFromPrompt: async (projectId, task, mainCheckout) => {
        const lane = await get().createLane(projectId, task, mainCheckout);
        if (!lane) return false;
        await get().create(lane.id, { prompt: task });
        return true;
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

      deleteLane: async (id) => {
        try {
          await backend.deleteLane(id);
          for (const session of get().sessions.filter((session) => session.laneId === id)) {
            forget(session.key);
          }
          await get().refresh();
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },

      dismissError: () => set({ error: null }),
    };
  });
}

export const useStore = createAppStore(api);
