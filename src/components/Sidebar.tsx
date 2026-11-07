import { AgentRow, Button, Checkbox, Dialog, Input, LoopMark } from "@panoptes/design-system";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { groupSessions, relativeTime, type SessionGroup } from "../grouping";
import { useStore } from "../store";
import type { ArchiveStrategy, Lane, Project, SidebarSession } from "../types";

/** Sessions shown per project before "more". */
const COLLAPSED_PROJECT_LIMIT = 5;

async function pickFolder(title: string): Promise<string | null> {
  const folder = await openDialog({ directory: true, title });
  return typeof folder === "string" ? folder : null;
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

export function Sidebar() {
  const sessions = useStore((s) => s.sessions);
  const drafts = useStore((s) => s.drafts);
  const listErrors = useStore((s) => s.listErrors);
  const projects = useStore((s) => s.projects);
  const lanes = useStore((s) => s.lanes);
  const create = useStore((s) => s.create);
  const addProject = useStore((s) => s.addProject);
  const archiveLane = useStore((s) => s.archiveLane);
  const [newLaneProject, setNewLaneProject] = useState<Project | null>(null);
  const [dirtyLane, setDirtyLane] = useState<Lane | null>(null);
  const [query, setQuery] = useState("");
  const groups = useMemo(
    () => groupSessions([...drafts, ...sessions], projects, query),
    [sessions, drafts, projects, query],
  );

  const startIn = async (cwd?: string, projectId?: number) => {
    const folder = cwd ?? (await pickFolder("Start a session in…"));
    if (folder) await create(folder, projectId);
  };

  const register = async () => {
    const folder = await pickFolder("Add a git repository as a project");
    if (folder) await addProject(folder);
  };

  const requestArchive = async (lane: Lane) => {
    const outcome = await archiveLane(lane.id, null);
    if (outcome?.type === "dirty") setDirtyLane(outcome.lane);
  };

  return (
    <aside className="flex w-72 shrink-0 flex-col border-r border-line bg-surface">
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
        <Button
          variant="ghost"
          size="sm"
          icon="plus"
          aria-label="New session"
          title="New session in a folder"
          onClick={() => void startIn()}
        />
      </header>
      <div className="p-3">
        <Input
          icon="search"
          placeholder="Filter sessions"
          aria-label="Filter sessions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      {listErrors.map((error) => (
        <p key={error} className="px-3 pb-2 text-error">
          {error}
        </p>
      ))}
      <nav className="flex-1 overflow-y-auto pb-4">
        {groups.map((group) => (
          <Group
            key={group.id}
            group={group}
            lanes={lanes.filter((lane) => lane.projectId === group.project?.id)}
            onNew={group.cwd ? () => void startIn(group.cwd, group.project?.id) : undefined}
            onNewLane={
              group.project ? () => setNewLaneProject(group.project as Project) : undefined
            }
            onArchive={(lane) => void requestArchive(lane)}
          />
        ))}
        {groups.length === 0 && (
          <p className="px-3 text-ink-muted">
            {query ? "No sessions match." : "No sessions yet. Start one with the + button."}
          </p>
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
    </aside>
  );
}

function Group({
  group,
  lanes,
  onNew,
  onNewLane,
  onArchive,
}: {
  group: SessionGroup;
  lanes: Lane[];
  onNew?: () => void;
  onNewLane?: () => void;
  onArchive: (lane: Lane) => void;
}) {
  const addProject = useStore((s) => s.addProject);
  const removeProject = useStore((s) => s.removeProject);
  const restoreLane = useStore((s) => s.restoreLane);
  const [expanded, setExpanded] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const active = lanes.filter((lane) => lane.state === "active");
  const archived = lanes.filter((lane) => lane.state === "archived");
  const project = group.project;
  const collapsible = group.cwd !== undefined;
  const shown =
    collapsible && !expanded ? group.sessions.slice(0, COLLAPSED_PROJECT_LIMIT) : group.sessions;
  const hidden = group.sessions.length - shown.length;

  return (
    <section className="group/section mt-2">
      <header className="flex h-7 items-center justify-between px-3 text-ink-muted">
        <h2 className="text-caption" title={group.cwd}>
          {group.label}
        </h2>
        <span className="invisible flex gap-1 group-hover/section:visible">
          {project && (
            <Button
              variant="ghost"
              size="sm"
              icon="x"
              aria-label={`Remove project ${group.label}`}
              title="Remove this project from Panoptes. Nothing on disk changes"
              onClick={() => void removeProject(project.id)}
            />
          )}
          {!project && group.checkout && (
            <Button
              variant="ghost"
              size="sm"
              icon="folder"
              aria-label={`Add ${group.label} as a project`}
              title={`Add ${group.checkout} as a project`}
              onClick={() => void addProject(group.checkout as string)}
            />
          )}
          {onNewLane && (
            <Button
              variant="ghost"
              size="sm"
              icon="git-branch"
              aria-label={`New lane in ${group.label}`}
              title={`New lane in ${group.label}`}
              onClick={onNewLane}
            />
          )}
          {onNew && (
            <Button
              variant="ghost"
              size="sm"
              icon="plus"
              aria-label={`New session in ${group.label}`}
              title={`New session in ${group.cwd}`}
              onClick={onNew}
            />
          )}
        </span>
      </header>
      {project && group.sessions.length === 0 && (
        <p className="px-3 pb-1 text-ink-muted">No sessions yet.</p>
      )}
      <ul>
        {active.map((lane) => (
          <LaneItem key={lane.id} lane={lane} onAction={() => onArchive(lane)} />
        ))}
        {showArchived &&
          archived.map((lane) => (
            <LaneItem key={lane.id} lane={lane} onAction={() => void restoreLane(lane.id)} />
          ))}
      </ul>
      {archived.length > 0 && (
        <Button
          variant="ghost"
          size="sm"
          className="ml-2"
          onClick={() => setShowArchived((shown) => !shown)}
        >
          {showArchived ? "Hide archived lanes" : `${archived.length} archived`}
        </Button>
      )}
      <ul>
        {shown.map((session) => (
          <SessionItem key={session.key} session={session} />
        ))}
      </ul>
      {hidden > 0 && (
        <Button variant="ghost" size="sm" className="ml-2" onClick={() => setExpanded(true)}>
          {hidden} more
        </Button>
      )}
    </section>
  );
}

/** An active lane offers Archive; an archived one offers Restore. */
function LaneItem({ lane, onAction }: { lane: Lane; onAction: () => void }) {
  const main = lane.worktrees.some((worktree) => worktree.mainCheckout);
  const archived = lane.state === "archived";
  const checkpoint = lane.worktrees.some((worktree) => worktree.checkpointRef);
  return (
    <li className={`group/lane relative px-3 py-2 ${archived ? "text-ink-muted" : ""}`}>
      <div className="pr-7">
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
      </div>
      <span className="absolute top-2 right-2 hidden group-hover/lane:block">
        <Button
          variant="ghost"
          size="sm"
          icon={archived ? "rotate-ccw" : "x"}
          aria-label={`${archived ? "Restore" : "Archive"} lane ${lane.name}`}
          title={
            archived
              ? checkpoint
                ? "Restore lane and its checkpoint"
                : "Restore lane"
              : "Archive lane"
          }
          onClick={onAction}
        />
      </span>
    </li>
  );
}

function NewLaneDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const createLane = useStore((s) => s.createLane);
  const cancelLaneCreation = useStore((s) => s.cancelLaneCreation);
  const [task, setTask] = useState("");
  const [mainCheckout, setMainCheckout] = useState(false);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!task.trim() || busy) return;
    setBusy(true);
    const lane = await createLane(project.id, task, mainCheckout);
    setBusy(false);
    if (lane) onClose();
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
          autoFocus
        />
        <Checkbox
          label="Use the main checkout"
          hint="Explicit opt-in: share the project's checkout. No worktree, copied files or setup command; archiving never removes it."
          checked={mainCheckout}
          onChange={(event) => setMainCheckout(event.target.checked)}
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

function SessionItem({ session }: { session: SidebarSession }) {
  const active = useStore((s) => s.activeKey === session.key);
  const status = useStore((s) => s.statuses[session.key] ?? "idle");
  const open = useStore((s) => s.open);
  const close = useStore((s) => s.close);
  const togglePin = useStore((s) => s.togglePin);

  return (
    <li className="group/item relative">
      <AgentRow
        name={session.title}
        task={basename(session.cwd)}
        status={status}
        time={relativeTime(session.updatedAt)}
        selected={active}
        onClick={() => void open(session)}
      />
      <span className="absolute top-1 right-2 hidden gap-1 rounded-sm bg-surface group-hover/item:flex">
        <Button variant="ghost" size="sm" onClick={() => void togglePin(session)}>
          {session.pinned ? "Unpin" : "Pin"}
        </Button>
        {session.open && (
          <Button
            variant="ghost"
            size="sm"
            title="Stop this session's agent process"
            onClick={() => void close(session.key)}
          >
            Stop
          </Button>
        )}
      </span>
    </li>
  );
}
