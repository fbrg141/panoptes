import { Button, Checkbox, Dialog, Input, LoopMark, Tabs } from "@panoptes/design-system";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { filterSessions } from "../grouping";
import { laneMenuItems, projectMenuItems } from "../sidebarMenu";
import { useStore } from "../store";
import type { ArchiveStrategy, Lane, LaneNavigation, Project, Session } from "../types";
import { RowMenu } from "./RowMenu";
import { SessionItem } from "./SessionItem";

type NavigationTab = "lanes" | "all";

async function pickFolder(title: string): Promise<string | null> {
  const folder = await openDialog({ directory: true, title });
  return typeof folder === "string" ? folder : null;
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
const includes = (text: string, query: string) => text.toLowerCase().includes(query);
const laneMatches = (lane: Lane, query: string) =>
  [lane.name, lane.task, lane.branch].some((text) => includes(text, query));
const sessionMatches = (session: Session, query: string) =>
  [session.title, session.cwd, ...session.tags].some((text) => includes(text, query));
const navigationMatches = (navigation: LaneNavigation | undefined, query: string) =>
  navigation !== undefined &&
  [...navigation.sessions, ...navigation.archivedSessions].some((session) =>
    sessionMatches(session, query),
  );

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const listErrors = useStore((s) => s.listErrors);
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const navigation = useStore((s) => s.navigation);
  const create = useStore((s) => s.create);
  const addProject = useStore((s) => s.addProject);
  const archiveLane = useStore((s) => s.archiveLane);
  const adopt = useStore((s) => s.adopt);
  const [tab, setTab] = useState<NavigationTab>("lanes");
  const [newLaneProject, setNewLaneProject] = useState<Project | null>(null);
  const [dirtyLane, setDirtyLane] = useState<Lane | null>(null);
  const [adopting, setAdopting] = useState<Session | null>(null);
  const [query, setQuery] = useState("");
  const navigationByLane = useMemo(
    () => new Map(navigation.map((entry) => [entry.laneId, entry])),
    [navigation],
  );
  const visibleSessions = useMemo(
    () => filterSessions(sessions, projects, lanes, query),
    [sessions, projects, lanes, query],
  );
  const needle = query.trim().toLowerCase();
  const visibleProjects = projects.filter((project) => {
    if (!needle || includes(project.name, needle) || includes(project.repo, needle)) return true;
    return lanes
      .filter((lane) => lane.projectId === project.id)
      .some(
        (lane) =>
          laneMatches(lane, needle) || navigationMatches(navigationByLane.get(lane.id), needle),
      );
  });

  const register = async () => {
    const folder = await pickFolder("Add a git repository as a project");
    if (folder) await addProject(folder);
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
          icon="folder"
          aria-label="Add project"
          title="Add a git repository as a project"
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
      <div className="p-3">
        <Input
          icon="search"
          placeholder={
            tab === "lanes" ? "Filter projects, lanes and sessions" : "Filter all sessions"
          }
          aria-label="Filter sessions"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      {listErrors.map((error) => (
        <p key={error} className="px-3 pb-2 text-error">
          {error}
        </p>
      ))}
      <nav
        className="flex-1 overflow-y-auto pb-4"
        aria-label={tab === "lanes" ? "Lanes" : "All sessions"}
      >
        {tab === "lanes" ? (
          <>
            {visibleProjects.map((project) => (
              <ProjectItem
                key={project.id}
                project={project}
                lanes={lanes.filter((lane) => lane.projectId === project.id)}
                navigationByLane={navigationByLane}
                query={needle}
                onNewLane={() => setNewLaneProject(project)}
                onWork={(lane) => void create(lane.id)}
                onArchive={(lane) => void requestArchive(lane)}
              />
            ))}
            {visibleProjects.length === 0 && (
              <p className="px-3 text-ink-muted">
                {needle ? "No lanes match." : "No projects yet. Add one with the folder button."}
              </p>
            )}
          </>
        ) : (
          <>
            <ul>
              {visibleSessions.map((session) => (
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
            {visibleSessions.length === 0 && (
              <p className="px-3 text-ink-muted">
                {needle ? "No sessions match." : "No harness sessions found."}
              </p>
            )}
          </>
        )}
      </nav>
      {newLaneProject && (
        <NewLaneDialog project={newLaneProject} onClose={() => setNewLaneProject(null)} />
      )}
      {dirtyLane && (
        <ArchiveLaneDialog
          lane={dirtyLane}
          onClose={() => setDirtyLane(null)}
          onArchived={() => setDirtyLane(null)}
        />
      )}
      {adopting && (
        <AdoptSessionDialog
          session={adopting}
          lanes={lanes.filter((lane) => lane.state === "active")}
          projects={projects}
          onClose={() => setAdopting(null)}
        />
      )}
    </aside>
  );
}

function ProjectItem({
  project,
  lanes,
  navigationByLane,
  query,
  onNewLane,
  onWork,
  onArchive,
}: {
  project: Project;
  lanes: Lane[];
  navigationByLane: Map<number, LaneNavigation>;
  query: string;
  onNewLane: () => void;
  onWork: (lane: Lane) => void;
  onArchive: (lane: Lane) => void;
}) {
  const removeProject = useStore((s) => s.removeProject);
  const restoreLane = useStore((s) => s.restoreLane);
  const [collapsed, setCollapsed] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const active = lanes.filter((lane) => lane.state === "active");
  const archived = lanes.filter((lane) => lane.state === "archived");
  const projectMatches =
    query !== "" && (includes(project.name, query) || includes(project.repo, query));
  const visible = (lane: Lane) =>
    !query ||
    projectMatches ||
    laneMatches(lane, query) ||
    navigationMatches(navigationByLane.get(lane.id), query);

  return (
    <section className="group/project mt-2">
      <header className="relative flex min-h-8 items-center px-2 text-ink-muted">
        <Button
          variant="ghost"
          size="sm"
          icon={collapsed && !query ? "chevron-right" : "chevron-down"}
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
        <RowMenu
          label={`Actions for project ${project.name}`}
          className="absolute top-1/2 right-2 hidden -translate-y-1/2 group-hover/project:block group-focus-within/project:block"
          items={projectMenuItems()}
          onSelect={(id) => {
            if (id === "new-lane") onNewLane();
            if (id === "remove-project") void removeProject(project.id);
          }}
        />
      </header>
      {(!collapsed || query) && (
        <div className="ml-3 border-l border-line pl-1">
          <ul>
            {active.filter(visible).map((lane) => (
              <LaneItem
                key={lane.id}
                lane={lane}
                navigation={navigationByLane.get(lane.id)}
                query={query}
                projectMatches={projectMatches}
                onWork={() => onWork(lane)}
                onAction={() => onArchive(lane)}
              />
            ))}
            {showArchived &&
              archived
                .filter(visible)
                .map((lane) => (
                  <LaneItem
                    key={lane.id}
                    lane={lane}
                    navigation={navigationByLane.get(lane.id)}
                    query={query}
                    projectMatches={projectMatches}
                    onAction={() => void restoreLane(lane.id)}
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
  query,
  projectMatches,
  onWork,
  onAction,
}: {
  lane: Lane;
  navigation?: LaneNavigation;
  query: string;
  projectMatches: boolean;
  onWork?: () => void;
  onAction: () => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const main = lane.worktrees.some((worktree) => worktree.mainCheckout);
  const archived = lane.state === "archived";
  const checkpoint = lane.worktrees.some((worktree) => worktree.checkpointRef);
  const directMatch = projectMatches || (query !== "" && laneMatches(lane, query));
  const sessions = navigation?.sessions ?? [];
  const archivedSessions = navigation?.archivedSessions ?? [];
  const visibleSessions = (items: Session[]) =>
    !query || directMatch ? items : items.filter((session) => sessionMatches(session, query));

  return (
    <li className={archived ? "text-ink-muted" : ""}>
      <div className="group/lane relative flex min-h-12 items-center px-1">
        <Button
          variant="ghost"
          size="sm"
          icon={collapsed && !query ? "chevron-right" : "chevron-down"}
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
        </button>
        <RowMenu
          label={`Actions for lane ${lane.name}`}
          className="absolute top-1/2 right-1 hidden -translate-y-1/2 group-hover/lane:block group-focus-within/lane:block"
          items={laneMenuItems(lane)}
          onSelect={(id) => {
            if (id === "new-session") onWork?.();
            if (id === "archive-lane" || id === "restore-lane") onAction();
          }}
        />
      </div>
      {(!collapsed || query) && (
        <div className="ml-4">
          <ul>
            {visibleSessions(archived ? [...sessions, ...archivedSessions] : sessions).map(
              (session) => (
                <SessionItem key={session.key} session={session} laneActions nested />
              ),
            )}
          </ul>
          {!archived && archivedSessions.length > 0 && (
            <>
              <Button variant="ghost" size="sm" onClick={() => setShowArchived((shown) => !shown)}>
                {showArchived
                  ? "Hide archived sessions"
                  : `${archivedSessions.length} archived sessions`}
              </Button>
              {showArchived && (
                <ul>
                  {visibleSessions(archivedSessions).map((session) => (
                    <SessionItem key={session.key} session={session} laneActions nested />
                  ))}
                </ul>
              )}
            </>
          )}
          {sessions.length === 0 && archivedSessions.length === 0 && (
            <p className="px-3 py-1 text-caption text-ink-muted">No sessions yet.</p>
          )}
        </div>
      )}
    </li>
  );
}

function NewLaneDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const createLane = useStore((s) => s.createLane);
  const createSession = useStore((s) => s.create);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const [task, setTask] = useState("");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [startSession, setStartSession] = useState(true);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!task.trim() || busy) return;
    setBusy(true);
    const lane = await createLane(project.id, task, mainCheckout);
    setBusy(false);
    if (lane) {
      onClose();
      if (startSession) await createSession(lane.id);
    }
  };

  const close = () => {
    if (busy) void cancelLaneCreation(project.id);
    onClose();
  };

  return (
    <Dialog
      title={`New lane in ${project.name}`}
      onClose={close}
      actions={
        <>
          <Button variant="secondary" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" form="new-lane" disabled={busy || !task.trim()}>
            {busy ? "Creating…" : "Create lane"}
          </Button>
        </>
      }
    >
      <form
        id="new-lane"
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Input
          label="Task"
          hint="A worktree, branch and write lock for this line of work."
          placeholder="Fix login redirects"
          aria-label="Lane task"
          value={task}
          onChange={(event) => setTask(event.target.value)}
          disabled={busy}
          autoFocus
        />
        <Checkbox
          label="Use the main checkout"
          hint="Explicit opt-in: share the project's checkout. No worktree, copied files or setup command; archiving never removes it."
          checked={mainCheckout}
          onChange={(event) => setMainCheckout(event.target.checked)}
          disabled={busy}
        />
        <Checkbox
          label="Start a session in this lane"
          hint="Open a new chat in its worktree as soon as the lane is ready."
          checked={startSession}
          onChange={(event) => setStartSession(event.target.checked)}
          disabled={busy}
        />
      </form>
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
        added.
      </p>
      {lanes.length === 0 ? (
        <p>Create an active lane before adopting this session.</p>
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
