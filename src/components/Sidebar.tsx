import { Button, Dialog, LoopMark, Tabs } from "@panoptes/design-system";
import { useEffect, useMemo, useRef, useState } from "react";
import { relativeTime } from "../grouping";
import { emptyLane, workLabel, writersLabel } from "../laneRow";
import { pickProjectFolder } from "../pickProject";
import { laneMenuItems, projectMenuItems } from "../sidebarMenu";
import { useStore } from "../store";
import type { ArchiveStrategy, Lane, LaneNavigation, Project, Session } from "../types";
import { useUi } from "../ui";
import { RowMenu } from "./RowMenu";
import { SessionItem, useClock } from "./SessionItem";

type NavigationTab = "lanes" | "all";

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const listErrors = useStore((s) => s.listErrors);
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const navigation = useStore((s) => s.navigation);
  const create = useStore((s) => s.create);
  const archiveLane = useStore((s) => s.archiveLane);
  const adopt = useStore((s) => s.adopt);
  const show = useUi((s) => s.open);
  const [tab, setTab] = useState<NavigationTab>("lanes");
  const [dirtyLane, setDirtyLane] = useState<Lane | null>(null);
  const [deletingLane, setDeletingLane] = useState<Lane | null>(null);
  const [continuing, setContinuing] = useState<Lane | null>(null);
  const [adopting, setAdopting] = useState<Session | null>(null);
  const navigationByLane = useMemo(
    () => new Map(navigation.map((entry) => [entry.laneId, entry])),
    [navigation],
  );

  const register = async () => {
    await pickProjectFolder();
  };

  const requestArchive = async (lane: Lane) => {
    const outcome = await archiveLane(lane.id, null);
    if (outcome?.type === "dirty") setDirtyLane(outcome.lane);
  };

  const requestAdoption = async (session: Session) => {
    const outcome = await adopt(session);
    if (outcome?.type === "chooseLane") setAdopting(session);
  };

  return (
    <aside className="flex w-80 shrink-0 flex-col border-r border-line bg-surface">
      <header className="flex h-[var(--titlebar)] items-center gap-2 border-b border-line px-3">
        <LoopMark size={20} />
        <span className="text-wordmark flex-1">Panoptes</span>
        <Button
          variant="ghost"
          size="sm"
          icon="search"
          aria-label="Search"
          title="Search projects, lanes and sessions (⌘P)"
          onClick={() => show({ type: "search" })}
        />
        <Button
          variant="ghost"
          size="sm"
          icon="command"
          aria-label="Keyboard shortcuts"
          title="Keyboard shortcuts (⌘/)"
          onClick={() => show({ type: "shortcuts" })}
        />
        <Button
          variant="ghost"
          size="sm"
          icon="folder"
          aria-label="Add project"
          title="Add a git repository as a project (⌘N)"
          onClick={() => void register()}
        />
      </header>
      <div className="px-3 pt-3">
        <Tabs
          tabs={[
            { id: "lanes", label: "Lanes", icon: "git-branch" },
            { id: "all", label: "All sessions", icon: "list", count: sessions.length },
          ]}
          value={tab}
          onChange={(id) => setTab(id as NavigationTab)}
        />
      </div>
      {listErrors.map((error) => (
        <p key={error} className="px-3 pb-2 pt-2 text-error">
          {error}
        </p>
      ))}
      <nav
        className="flex-1 overflow-y-auto pb-4"
        aria-label={tab === "lanes" ? "Lanes" : "All sessions"}
      >
        {tab === "lanes" ? (
          <>
            {projects.map((project) => (
              <ProjectItem
                key={project.id}
                project={project}
                lanes={lanes.filter((lane) => lane.projectId === project.id)}
                navigationByLane={navigationByLane}
                onNewLane={() => show({ type: "newLane", projectId: project.id })}
                onWork={(lane) => void create(lane.id)}
                onContinue={setContinuing}
                onArchive={(lane) => void requestArchive(lane)}
                onDelete={setDeletingLane}
              />
            ))}
            {projects.length === 0 && (
              <p className="px-3 text-ink-muted">
                No projects yet. Add one with the folder button.
              </p>
            )}
          </>
        ) : (
          <>
            <ul>
              {sessions.map((session) => (
                <SessionItem
                  key={session.key}
                  session={session}
                  context={sessionContext(session, projects, lanes)}
                  onAdopt={
                    session.laneId === null ? () => void requestAdoption(session) : undefined
                  }
                />
              ))}
            </ul>
            {sessions.length === 0 && (
              <p className="px-3 text-ink-muted">No harness sessions found.</p>
            )}
          </>
        )}
      </nav>
      {dirtyLane && (
        <ArchiveLaneDialog
          lane={dirtyLane}
          onClose={() => setDirtyLane(null)}
          onArchived={() => setDirtyLane(null)}
        />
      )}
      {deletingLane && (
        <DeleteLaneDialog lane={deletingLane} onClose={() => setDeletingLane(null)} />
      )}
      {continuing && <HandoffDialog lane={continuing} onClose={() => setContinuing(null)} />}
      {adopting && (
        <AdoptSessionDialog
          session={adopting}
          lanes={lanes.filter((lane) => {
            if (lane.state !== "active") return false;
            const entry = navigationByLane.get(lane.id);
            return !entry || (entry.sessions.length === 0 && entry.archivedSessions.length === 0);
          })}
          projects={projects}
          onClose={() => setAdopting(null)}
        />
      )}
    </aside>
  );
}

/**
 * A row's menu trigger stays in the layout and in the tab order, and shows on
 * hover, on keyboard focus within the row, and while its menu is open.
 */
const REVEAL_ON_HOVER =
  "opacity-0 transition-opacity duration-[var(--duration-fast)] has-[[aria-expanded=true]]:opacity-100";

function ProjectItem({
  project,
  lanes,
  navigationByLane,
  onNewLane,
  onWork,
  onContinue,
  onArchive,
  onDelete,
}: {
  project: Project;
  lanes: Lane[];
  navigationByLane: Map<number, LaneNavigation>;
  onNewLane: () => void;
  onWork: (lane: Lane) => void;
  onContinue: (lane: Lane) => void;
  onArchive: (lane: Lane) => void;
  onDelete: (lane: Lane) => void;
}) {
  const removeProject = useStore((s) => s.removeProject);
  const restoreLane = useStore((s) => s.restoreLane);
  const [collapsed, setCollapsed] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const active = lanes.filter((lane) => lane.state === "active");
  const archived = lanes.filter((lane) => lane.state === "archived");

  return (
    <section className="group/project mt-2">
      <header className="flex min-h-8 items-center px-2 text-ink-muted">
        <Button
          variant="ghost"
          size="sm"
          icon={collapsed ? "chevron-right" : "chevron-down"}
          aria-label={`${collapsed ? "Expand" : "Collapse"} project ${project.name}`}
          onClick={() => setCollapsed((value) => !value)}
        />
        <button
          type="button"
          className="min-w-0 flex-1 truncate py-1 text-left text-caption"
          title={project.repo}
          onClick={() => setCollapsed((value) => !value)}
        >
          {project.name}
        </button>
        <Button
          variant="ghost"
          size="sm"
          icon="plus"
          aria-label={`New lane in ${project.name}`}
          title={`New lane in ${project.name}`}
          className={`${REVEAL_ON_HOVER} group-hover/project:opacity-100 group-focus-within/project:opacity-100`}
          onClick={onNewLane}
        />
        <RowMenu
          label={`Actions for project ${project.name}`}
          className={`ml-1 flex flex-none ${REVEAL_ON_HOVER} group-hover/project:opacity-100 group-focus-within/project:opacity-100`}
          items={projectMenuItems()}
          onSelect={(id) => {
            if (id === "new-lane") onNewLane();
            if (id === "remove-project") void removeProject(project.id);
          }}
        />
      </header>
      {!collapsed && (
        <div className="ml-3 border-l border-line pl-1">
          <ul>
            {active.map((lane) => (
              <LaneItem
                key={lane.id}
                lane={lane}
                navigation={navigationByLane.get(lane.id)}
                onWork={() => onWork(lane)}
                onContinue={() => onContinue(lane)}
                onAction={() => onArchive(lane)}
                onDelete={() => onDelete(lane)}
              />
            ))}
            {showArchived &&
              archived.map((lane) => (
                <LaneItem
                  key={lane.id}
                  lane={lane}
                  navigation={navigationByLane.get(lane.id)}
                  onAction={() => void restoreLane(lane.id)}
                  onDelete={() => onDelete(lane)}
                />
              ))}
          </ul>
          {active.length === 0 && <p className="px-3 py-1 text-ink-muted">No active lanes.</p>}
          {archived.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="ml-2"
              onClick={() => setShowArchived((shown) => !shown)}
            >
              {showArchived ? "Hide archived lanes" : `${archived.length} archived lanes`}
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

/** A lane and its backend-assigned sessions, both independently collapsible. */
function LaneItem({
  lane,
  navigation,
  onWork,
  onContinue,
  onAction,
  onDelete,
}: {
  lane: Lane;
  navigation?: LaneNavigation;
  onWork?: () => void;
  onContinue?: () => void;
  onAction: () => void;
  onDelete: () => void;
}) {
  const allSessions = useStore((s) => s.sessions);
  const allLanes = useStore((s) => s.lanes);
  const openSession = useStore((s) => s.open);
  const now = useClock();
  const empty = emptyLane(lane, navigation);
  const work = workLabel(navigation?.work ?? null);
  const writers = navigation?.writers ?? [];
  const writerLine = writersLabel(writers, (at) => relativeTime(at, Math.max(now, at)));
  const latestWriter = allSessions.find((session) => session.key === writers[0]?.sessionKey);
  const [collapsed, setCollapsed] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const main = lane.worktrees.some((worktree) => worktree.mainCheckout);
  const archived = lane.state === "archived";
  const checkpoint = lane.worktrees.some((worktree) => worktree.checkpointRef);
  const sessions = navigation?.sessions ?? [];
  const archivedSessions = navigation?.archivedSessions ?? [];

  return (
    <li id={laneElementId(lane.id)} tabIndex={-1} className={archived ? "text-ink-muted" : ""}>
      <div className="group/lane relative flex min-h-12 items-center px-1">
        <Button
          variant="ghost"
          size="sm"
          icon={collapsed ? "chevron-right" : "chevron-down"}
          aria-label={`${collapsed ? "Expand" : "Collapse"} lane ${lane.name}`}
          onClick={() => setCollapsed((value) => !value)}
        />
        <button
          type="button"
          className="min-w-0 flex-1 py-1 pr-8 text-left"
          onClick={() => setCollapsed((value) => !value)}
        >
          <p className="truncate" title={lane.task}>
            {lane.task}
          </p>
          <p
            className={`truncate text-caption ${main ? "text-waiting" : "text-ink-muted"}`}
            title={lane.worktrees[0]?.path}
          >
            {main ? "Main checkout" : lane.branch}
            {archived && (checkpoint ? " · archived with a checkpoint" : " · archived")}
          </p>
          {work && <p className="truncate text-xs text-ink-muted">{work}</p>}
        </button>
        <RowMenu
          label={`Actions for lane ${lane.name}`}
          className={`absolute inset-y-0 right-1 flex items-center ${REVEAL_ON_HOVER} group-hover/lane:opacity-100 group-focus-within/lane:opacity-100`}
          items={laneMenuItems(lane)}
          onSelect={(id) => {
            if (id === "archive-lane" || id === "restore-lane") onAction();
            if (id === "delete-lane") onDelete();
          }}
        />
      </div>
      {!collapsed && (
        <div className="ml-4">
          {writerLine && (
            // Another session changed this lane: name it once, as a link to
            // that session's own row, never as a second row here.
            <button
              type="button"
              className={`block w-full truncate py-0.5 pr-2 pl-3.5 text-left text-xs hover:text-ink ${
                writers[0]?.live ? "text-waiting" : "text-ink-muted"
              }`}
              title={latestWriter ? `Open “${latestWriter.title}”` : undefined}
              disabled={!latestWriter}
              onClick={() => latestWriter && void openSession(latestWriter)}
            >
              {writerLine}
            </button>
          )}
          <ul>
            {(archived ? [...sessions, ...archivedSessions] : sessions).map((session) => (
              <SessionItem key={session.key} session={session} laneActions nested />
            ))}
          </ul>
          {sessions.flatMap((session) =>
            session.editedLanes.map((id) => {
              const other = allLanes.find((item) => item.id === id);
              if (!other) return null;
              return (
                <button
                  key={`${session.key}-${id}`}
                  type="button"
                  className="block w-full truncate py-0.5 pr-2 pl-10 text-left text-xs text-ink-muted hover:text-ink"
                  title={`Show the lane “${other.task}”`}
                  onClick={() => revealLane(id)}
                >
                  Also edited: {other.task}
                </button>
              );
            }),
          )}
          {!archived && archivedSessions.length > 0 && (
            <>
              <Button variant="ghost" size="sm" onClick={() => setShowArchived((shown) => !shown)}>
                {showArchived
                  ? "Hide archived sessions"
                  : `${archivedSessions.length} archived sessions`}
              </Button>
              {showArchived && (
                <ul>
                  {archivedSessions.map((session) => (
                    <SessionItem key={session.key} session={session} laneActions nested />
                  ))}
                </ul>
              )}
            </>
          )}
          {empty?.kind === "notStarted" && onWork && (
            <Button variant="ghost" size="sm" className="ml-2" onClick={onWork}>
              Start session
            </Button>
          )}
          {empty?.kind === "hasWork" && onContinue && (
            <Button
              variant="ghost"
              size="sm"
              className="ml-2"
              title="Start this lane's session with a brief of the work already here"
              onClick={onContinue}
            >
              Continue in this lane
            </Button>
          )}
          {empty?.kind === "beingWritten" && (
            <p className="ml-2 py-1 text-xs text-ink-muted">
              No session here yet. Continue once{" "}
              {empty.writer.title ? `“${empty.writer.title}”` : "the other session"} is Done.
            </p>
          )}
        </div>
      )}
    </li>
  );
}

const laneElementId = (id: number) => `lane-${id}`;

/** Scrolls a lane's row into view and moves focus to it. */
function revealLane(id: number) {
  const row = document.getElementById(laneElementId(id));
  row?.scrollIntoView({ block: "nearest" });
  row?.focus();
}

/**
 * Continue in this lane: its own session starts with a brief built from what
 * the lane holds and which conversations started it, shown here to edit
 * before anything is sent.
 */
function HandoffDialog({ lane, onClose }: { lane: Lane; onClose: () => void }) {
  const handoffBrief = useStore((s) => s.handoffBrief);
  const create = useStore((s) => s.create);
  const [brief, setBrief] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Read once per lane; a parent re-render must not replace an edited brief.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    let current = true;
    void handoffBrief(lane.id).then((text) => {
      if (!current) return;
      if (text === null) closeRef.current();
      else setBrief(text);
    });
    return () => {
      current = false;
    };
  }, [handoffBrief, lane.id]);

  const send = async () => {
    const text = brief?.trim();
    if (!text || busy) return;
    setBusy(true);
    await create(lane.id, { prompt: text });
    onClose();
  };

  return (
    <Dialog
      title={`Continue “${lane.task}” in this lane`}
      width={640}
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={busy || !brief?.trim()} onClick={() => void send()}>
            {busy ? "Starting…" : "Start session"}
          </Button>
        </>
      }
    >
      <p className="pb-3 text-ink-muted">
        This lane's own session starts with this brief. It lists only what git and Panoptes
        recorded, so check it against the work and add anything the agent should know.
      </p>
      {brief === null ? (
        <p className="text-ink-muted">Reading the lane…</p>
      ) : (
        <textarea
          aria-label="Brief for the new session"
          className="text-code min-h-64 w-full rounded-sm border border-line-strong bg-surface-sunken p-2 text-ink focus:border-focus focus:outline-2 focus:outline-offset-1 focus:outline-focus"
          value={brief}
          disabled={busy}
          onChange={(event) => setBrief(event.target.value)}
        />
      )}
    </Dialog>
  );
}

function DeleteLaneDialog({ lane, onClose }: { lane: Lane; onClose: () => void }) {
  const deleteLane = useStore((s) => s.deleteLane);
  const [busy, setBusy] = useState(false);
  const main = lane.worktrees.some((worktree) => worktree.mainCheckout);

  const remove = async () => {
    if (busy) return;
    setBusy(true);
    if (await deleteLane(lane.id)) onClose();
    else setBusy(false);
  };

  return (
    <Dialog
      title={`Delete ${lane.name}?`}
      width={540}
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={busy} onClick={() => void remove()}>
            Delete permanently
          </Button>
        </>
      }
    >
      {main ? (
        <p>
          This removes the lane and its Panoptes session records. The main checkout, its branch and
          your files are not changed. Pi conversation files are kept.
        </p>
      ) : (
        <p>
          This permanently deletes the local worktree, including uncommitted changes, the local
          branch <code>{lane.branch}</code>, and this lane's recovery refs. Commits that exist only
          on this branch will be lost. The main repository, remote branches and Pi conversation
          files are kept.
        </p>
      )}
    </Dialog>
  );
}

function ArchiveLaneDialog({
  lane,
  onClose,
  onArchived,
}: {
  lane: Lane;
  onClose: () => void;
  onArchived: () => void;
}) {
  const archiveLane = useStore((s) => s.archiveLane);
  const [busy, setBusy] = useState(false);

  const archive = async (strategy: ArchiveStrategy) => {
    if (busy) return;
    setBusy(true);
    const outcome = await archiveLane(lane.id, strategy);
    setBusy(false);
    if (outcome?.type === "archived") onArchived();
  };

  return (
    <Dialog
      title={`Preserve ${lane.name} before archiving`}
      width={540}
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button variant="secondary" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button variant="secondary" disabled={busy} onClick={() => void archive("commit")}>
            Commit
          </Button>
          <Button variant="secondary" disabled={busy} onClick={() => void archive("stash")}>
            Stash
          </Button>
          <Button disabled={busy} onClick={() => void archive("checkpoint")}>
            Keep as checkpoint
          </Button>
        </>
      }
    >
      <p>
        This lane has uncommitted changes. Nothing has been changed or removed yet. Commit them on{" "}
        <code>{lane.branch}</code>, stash them on that branch, or save a private Panoptes checkpoint
        that can be restored later.
      </p>
    </Dialog>
  );
}

function AdoptSessionDialog({
  session,
  lanes,
  projects,
  onClose,
}: {
  session: Session;
  lanes: Lane[];
  projects: Project[];
  onClose: () => void;
}) {
  const adopt = useStore((s) => s.adopt);
  const [busy, setBusy] = useState(false);
  const choose = async (laneId: number) => {
    if (busy) return;
    setBusy(true);
    const outcome = await adopt(session, laneId);
    setBusy(false);
    if (outcome?.type === "adopted") onClose();
  };

  return (
    <Dialog title={`Adopt “${session.title}” into a lane`} onClose={busy ? undefined : onClose}>
      <p className="pb-3 text-ink-muted">
        The conversation stays in the harness's own files. Only its Panoptes lane membership is
        added. Each lane can hold one session.
      </p>
      {lanes.length === 0 ? (
        <p>No active lane without a session is available. Create a lane for a new task instead.</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {lanes.map((lane) => (
            <li key={lane.id}>
              <Button
                variant="ghost"
                className="w-full justify-start"
                disabled={busy}
                onClick={() => void choose(lane.id)}
              >
                {projects.find((project) => project.id === lane.projectId)?.name} / {lane.task}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}

function sessionContext(session: Session, projects: Project[], lanes: Lane[]): string {
  const lane = lanes.find((item) => item.id === session.laneId);
  if (lane) {
    const project = projects.find((item) => item.id === lane.projectId);
    return `${project?.name ?? "Project"} / ${lane.task}`;
  }
  const project = projects.find((item) => item.id === session.projectId);
  return project?.name ?? basename(session.projectRoot);
}
