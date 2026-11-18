import { create } from "zustand";
import { api } from "./api";
import { applyEvent, type Conversation, emptyConversation, removeQuestion } from "./conversation";
import type {
  AdoptionOutcome,
  Answer,
  AppInfo,
  ArchiveLaneOutcome,
  ArchiveStrategy,
  DeleteLaneOutcome,
  Draft,
  ErrorKind,
  ImageAttachment,
  Lane,
  LaneNavigation,
  ModelInfo,
  OpenedSession,
  Preferences,
  ProfileInput,
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

export interface SessionOptions {
  model?: string;
  thinking?: string;
  /** Stored with a new session and appended again when its process resumes. */
  instructions?: string;
  prompt?: string;
}

export interface ArchiveLanesResult {
  archived: number;
  failed: { laneId: number; message: string }[];
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
  /** A failed first open, shown in its pane instead of loading forever. */
  openErrors: Record<string, string>;
  /** Each open session's status, as the backend reports it. */
  statuses: Record<string, SessionStatus>;
  /** Keys with a prompt on its way to the harness. */
  sending: Record<string, boolean>;
  /** Model, thinking level and context use, per open session. */
  settings: Record<string, SessionSettings>;
  /** Models each open session can switch to, once asked for. */
  models: Record<string, ModelInfo[]>;
  /** Panoptes' profiles and model-picker presentation. */
  preferences: Preferences;
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
  create: (laneId: number, options?: SessionOptions) => Promise<void>;
  /** Adopts a discovered conversation into a matching or explicitly chosen lane. */
  adopt: (session: Session, laneId?: number | null) => Promise<AdoptionOutcome | null>;
  /** Hides a lane session (closing it first) or brings it back; its lane is untouched. */
  setArchived: (key: string, archived: boolean) => Promise<void>;
  /** Renames a lane session; an empty title falls back to the harness's. */
  rename: (key: string, title: string) => Promise<void>;
  /** Resolves to whether the harness took the prompt, so a caller can keep its draft. */
  send: (text: string, options?: SendOptions, sessionKey?: string) => Promise<boolean>;
  /** Keeps a composer draft under its session, or drops it once it is empty. */
  saveDraft: (key: string, draft: Draft) => void;
  loadModels: (sessionKey?: string) => Promise<boolean>;
  saveProfile: (id: number | null, input: ProfileInput) => Promise<boolean>;
  deleteProfile: (id: number) => Promise<void>;
  setModelFavorite: (modelId: string, favorite: boolean) => Promise<void>;
  setModelHidden: (modelId: string, hidden: boolean) => Promise<void>;
  setCleanupIdleDays: (days: number) => Promise<boolean>;
  setModel: (modelId: string, sessionKey?: string) => Promise<void>;
  setThinkingLevel: (level: string, sessionKey?: string) => Promise<void>;
  abort: (sessionKey?: string) => Promise<void>;
  answer: (questionId: string, answer: Answer, sessionKey?: string) => Promise<void>;
  close: (key: string) => Promise<void>;
  /** Registers the git repository `folder` is in. */
  addProject: (folder: string) => Promise<void>;
  /** Removes a project after explicit confirmation in the UI. */
  removeProject: (id: number) => Promise<boolean>;
  createLane: (projectId: number, task: string, mainCheckout: boolean) => Promise<Lane | null>;
  /** Creates a lane and starts its only session, configured by `options`. */
  createFromPrompt: (
    projectId: number,
    task: string,
    mainCheckout: boolean,
    options?: Omit<SessionOptions, "prompt">,
  ) => Promise<boolean>;
  cancelLaneCreation: (projectId: number) => Promise<void>;
  archiveLane: (
    id: number,
    strategy: ArchiveStrategy | null,
    recreateBranch?: boolean,
  ) => Promise<ArchiveLaneOutcome | null>;
  /** Archives independently, preserving any dirty selected lane as a checkpoint. */
  archiveLanes: (ids: number[]) => Promise<ArchiveLanesResult>;
  /** Saves a detached HEAD on the lane branch, leaving the worktree in place. */
  createLaneBranch: (id: number) => Promise<boolean>;
  restoreLane: (id: number) => Promise<void>;
  /** Permanently deletes a lane after explicit confirmation in the UI. */
  deleteLane: (id: number, allowDetached?: boolean) => Promise<DeleteLaneOutcome | null>;
  /** Opens the event log folder in Finder. */
  showLogs: () => Promise<void>;
  dismissError: () => void;
}

/** Commands reject with a `ServiceError`; anything else is shown as it is. */
export function describeError(error: unknown): string {
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

/** Nothing was sent to the absent process, so retrying cannot duplicate it. */
const canRetryPrompt = (error: unknown) => errorKind(error) === "notOpen";

/** Builds the store around an API, so tests can pass a fake one. */
export function createAppStore(backend: typeof api) {
  let listening = false;

  return create<State>((set, get) => {
    const fail = (error: unknown) => set({ error: describeError(error) });

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
      patch(opened.key, (c) =>
        // A reloaded window catches up on the run and on what is still
        // waiting for an answer; anything it already has is kept once.
        opened.replay.reduce(applyEvent, {
          ...c,
          messages: historyApplies(opened.key, epoch, c) ? opened.history : c.messages,
          exited: null,
        }),
      );
      // A status that arrived while opening is newer than this one.
      set((s) =>
        opened.key in s.statuses
          ? {}
          : { statuses: { ...s.statuses, [opened.key]: opened.status } },
      );
      void loadSettings(opened.key);
    };

    /** Drops everything kept about a session that is no longer open. */
    const forget = (key: string) =>
      set((s) => {
        const { [key]: _closed, ...conversations } = s.conversations;
        const { [key]: _openError, ...openErrors } = s.openErrors;
        const { [key]: _status, ...statuses } = s.statuses;
        const { [key]: _settings, ...settings } = s.settings;
        const { [key]: _models, ...models } = s.models;
        const { [key]: _draft, ...drafts } = s.drafts;
        epochs.delete(key);
        return {
          conversations,
          openErrors,
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
      openErrors: {},
      statuses: {},
      sending: {},
      settings: {},
      models: {},
      preferences: { profiles: [], favoriteModels: [], hiddenModels: [], cleanupIdleDays: 4 },
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
        try {
          set({ preferences: await backend.preferences() });
        } catch (e) {
          fail(e);
        }
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
        set((s) => {
          const { [session.key]: _openError, ...openErrors } = s.openErrors;
          return { activeKey: session.key, openErrors };
        });
        const epoch = bump(session.key);
        try {
          const opened = await backend.openSession(session);
          // An adopted conversation answers with its lane key, which has
          // its own history of events.
          adopt(opened, opened.key === session.key ? epoch : undefined);
          await get().refresh();
        } catch (e) {
          // A reopened session keeps its transcript. A first open needs its
          // own error state, or the pane would show "Starting" forever.
          if (!get().conversations[session.key] && epochs.get(session.key) === epoch) {
            const message = describeError(e);
            set((s) => ({ openErrors: { ...s.openErrors, [session.key]: message } }));
          }
          fail(e);
        }
      },

      create: async (laneId, options) => {
        try {
          // The session is recorded before it starts, so it is listed at once.
          const opened = await backend.newSession(laneId, "pi", options?.instructions);
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

      send: async (text, { images = [], queue = false } = {}, sessionKey) => {
        const key = sessionKey ?? get().activeKey;
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
          // Retry once only when nothing was sent. If the process exited
          // after receiving the prompt, reopening and resending could duplicate it.
          // A queued prompt shows once the harness reports its queue.
          const outcome = await prompt().catch(async (e) => {
            if (!canRetryPrompt(e)) throw e;
            await reopen();
            return prompt();
          });
          if (outcome === "started") patch(key, (c) => ({ ...c, busy: true }));
          if (outcome === "handled") void reloadHistory(key);
          return true;
        } catch (e) {
          if (errorKind(e) === "exited") {
            // The prompt may have arrived before the process ended, so the
            // person checks the conversation rather than resending blindly.
            set({
              error: `${describeError(e)}. Check whether your prompt arrived before sending it again.`,
            });
          } else {
            fail(e);
          }
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

      loadModels: async (sessionKey) => {
        const key = sessionKey ?? get().activeKey;
        if (!key) return false;
        try {
          const models = await backend.listModels(key);
          set((s) => ({ models: { ...s.models, [key]: models } }));
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },

      saveProfile: async (id, input) => {
        try {
          set({ preferences: await backend.saveProfile(id, input) });
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },

      deleteProfile: async (id) => {
        try {
          set({ preferences: await backend.deleteProfile(id) });
        } catch (e) {
          fail(e);
        }
      },

      setModelFavorite: async (modelId, favorite) => {
        try {
          set({ preferences: await backend.setModelFavorite(modelId, favorite) });
        } catch (e) {
          fail(e);
        }
      },

      setModelHidden: async (modelId, hidden) => {
        try {
          set({ preferences: await backend.setModelHidden(modelId, hidden) });
        } catch (e) {
          fail(e);
        }
      },

      setCleanupIdleDays: async (days) => {
        try {
          set({ preferences: await backend.setCleanupIdleDays(days) });
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },

      setModel: async (modelId, sessionKey) => {
        const key = sessionKey ?? get().activeKey;
        if (!key) return;
        try {
          const settings = await backend.setModel(key, modelId);
          set((s) => ({ settings: { ...s.settings, [key]: settings } }));
        } catch (e) {
          fail(e);
        }
      },

      setThinkingLevel: async (level, sessionKey) => {
        const key = sessionKey ?? get().activeKey;
        if (!key) return;
        try {
          const settings = await backend.setThinkingLevel(key, level);
          set((s) => ({ settings: { ...s.settings, [key]: settings } }));
        } catch (e) {
          fail(e);
        }
      },

      abort: async (sessionKey) => {
        const key = sessionKey ?? get().activeKey;
        if (key) await backend.abort(key).catch(fail);
      },

      answer: async (questionId, answer, sessionKey) => {
        // Inline approvals in a transcript do not carry a pane prop; their id
        // still identifies the owning conversation when another pane is focused.
        const key =
          sessionKey ??
          Object.entries(get().conversations).find(
            ([, conversation]) =>
              conversation.questions.some((question) => question.id === questionId) ||
              conversation.approvals.some((approval) => approval.id === questionId),
          )?.[0] ??
          get().activeKey;
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
          return true;
        } catch (e) {
          fail(e);
          return false;
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

      createFromPrompt: async (projectId, task, mainCheckout, options) => {
        const lane = await get().createLane(projectId, task, mainCheckout);
        if (!lane) return false;
        await get().create(lane.id, { ...options, prompt: task });
        return true;
      },

      cancelLaneCreation: async (projectId) => {
        await backend.cancelLaneCreation(projectId).catch(fail);
      },

      archiveLane: async (id, strategy, recreateBranch = false) => {
        try {
          const outcome = await backend.archiveLane(id, strategy, recreateBranch);
          if (outcome.type === "archived") await get().refresh();
          return outcome;
        } catch (e) {
          fail(e);
          return null;
        }
      },

      archiveLanes: async (ids) => {
        let archived = 0;
        const failed: { laneId: number; message: string }[] = [];
        // Different lanes can share a repository. Archive sequentially rather
        // than racing their Git housekeeping and checkpoint writes.
        for (const laneId of ids) {
          try {
            const outcome = await backend.archiveLane(laneId, "checkpoint");
            if (outcome.type === "archived") archived += 1;
            else if (outcome.type === "detachedHead") {
              failed.push({
                laneId,
                message: "Needs branch recovery; archive it from its lane menu.",
              });
            } else {
              failed.push({
                laneId,
                message: "Its worktree changed. Scan again before archiving it.",
              });
            }
          } catch (error) {
            failed.push({ laneId, message: describeError(error) });
          }
        }
        await get().refresh();
        return { archived, failed };
      },

      createLaneBranch: async (id) => {
        try {
          await backend.createLaneBranch(id);
          await get().refresh();
          return true;
        } catch (e) {
          fail(e);
          return false;
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

      deleteLane: async (id, allowDetached = false) => {
        try {
          const outcome = await backend.deleteLane(id, allowDetached);
          if (outcome.type === "deleted") {
            for (const session of get().sessions.filter((session) => session.laneId === id)) {
              forget(session.key);
            }
            await get().refresh();
          }
          return outcome;
        } catch (e) {
          fail(e);
          return null;
        }
      },

      showLogs: async () => {
        try {
          await backend.showLogs();
        } catch (e) {
          fail(e);
        }
      },

      dismissError: () => set({ error: null }),
    };
  });
}

export const useStore = createAppStore(api);
