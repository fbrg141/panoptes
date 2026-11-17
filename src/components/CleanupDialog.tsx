import { Button, Checkbox, Icon } from "@panoptes/design-system";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { formatBytes, idleLabel, usageLabel } from "../cleanup";
import { describeError, useStore } from "../store";
import type { LaneCleanup } from "../types";
import { useUi } from "../ui";
import { Overlay, OverlayFooter, OverlayHeader } from "./Overlay";

type Outcome = {
  archived: number;
  failed: { laneId: number; text: string }[];
};

/** Project-scoped worktree usage and explicit, checkpoint-safe bulk archive. */
export function CleanupDialog({ projectId }: { projectId: number }) {
  const close = useUi((s) => s.close);
  const project = useStore((s) => s.projects.find((item) => item.id === projectId));
  const lanes = useStore((s) => s.lanes);
  const archiveLanes = useStore((s) => s.archiveLanes);
  const [data, setData] = useState<LaneCleanup | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const generation = useRef(0);

  const load = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true);
    setError(null);
    try {
      const result = await api.laneCleanup(projectId);
      if (generation.current !== request) return;
      setData(result);
      const available = new Set(result.candidates.map((candidate) => candidate.lane.id));
      setSelected((current) => new Set([...current].filter((id) => available.has(id))));
    } catch (cause) {
      if (generation.current === request) setError(describeError(cause));
    } finally {
      if (generation.current === request) setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    void load();
    return () => {
      generation.current += 1;
    };
  }, [load]);

  const toggle = (laneId: number, checked: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      if (checked) next.add(laneId);
      else next.delete(laneId);
      return next;
    });
    setOutcome(null);
  };

  const archive = async () => {
    if (busy || selected.size === 0 || !data) return;
    setBusy(true);
    setOutcome(null);
    const result = await archiveLanes([...selected]);
    const failed = result.failed.map((failure) => {
      const task =
        data.candidates.find((candidate) => candidate.lane.id === failure.laneId)?.lane.task ??
        lanes.find((lane) => lane.id === failure.laneId)?.task ??
        `Lane ${failure.laneId}`;
      return { laneId: failure.laneId, text: `${task}: ${failure.message}` };
    });
    setBusy(false);
    setConfirming(false);
    setSelected(new Set(result.failed.map((failure) => failure.laneId)));
    setOutcome({ archived: result.archived, failed });
    await load();
  };

  return (
    <Overlay
      label={`Clean up lanes in ${project?.name ?? "project"}`}
      width={620}
      onClose={busy ? undefined : close}
    >
      <OverlayHeader onClose={busy ? undefined : close}>
        <h2 className="text-heading">Clean up {project?.name ?? "lanes"}</h2>
        {loading && data && <span className="text-label text-ink-muted">Refreshing…</span>}
      </OverlayHeader>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {loading && !data && (
          <p className="py-6 text-center text-ink-muted">Measuring worktrees…</p>
        )}
        {error && (
          <div className="flex flex-col items-start gap-3 py-4" role="alert">
            <p className="text-error">{error}</p>
            <Button variant="secondary" icon="refresh-cw" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        )}
        {data && (
          <div className="flex flex-col gap-4">
            <div className="rounded-md border border-line px-3 py-2">
              <div className="flex items-center justify-between gap-3 py-1">
                <span className="text-ink-muted">This project</span>
                <span className="text-body-strong">{usageLabel(data.project)}</span>
              </div>
              <div className="flex items-center justify-between gap-3 border-t border-line py-1">
                <span className="text-ink-muted">All projects</span>
                <span className="text-body-strong">{usageLabel(data.total)}</span>
              </div>
            </div>
            {data.errors.length > 0 && (
              <div className="flex items-start gap-2 text-error" role="alert">
                <Icon name="circle-alert" size={16} className="mt-0.5 flex-none" />
                <div>
                  <p>Some cleanup information is unavailable.</p>
                  {data.errors.map((message) => (
                    <p key={message} className="mt-1 text-code-sm">
                      {message}
                    </p>
                  ))}
                </div>
              </div>
            )}
            {outcome && (
              <div role="status">
                {outcome.archived > 0 && (
                  <p className="text-done">
                    Archived {outcome.archived} {outcome.archived === 1 ? "lane" : "lanes"}.
                  </p>
                )}
                <div className="text-error">
                  {outcome.failed.map((failure) => (
                    <p key={failure.laneId} className="mt-1">
                      {failure.text}
                    </p>
                  ))}
                </div>
              </div>
            )}
            <section>
              <h3 className="text-body-strong">Idle for {data.idleDays} or more days</h3>
              <p className="mt-1 text-label font-normal text-ink-muted">
                Only lanes with no open sessions that can be archived without branch recovery are
                listed. Nothing is selected by default.
              </p>
              {data.candidates.length === 0 ? (
                <p className="py-5 text-center text-ink-muted">No safe idle lanes to archive.</p>
              ) : (
                <ul className="mt-3 flex flex-col gap-2">
                  {data.candidates.map((candidate) => {
                    const id = candidate.lane.id;
                    return (
                      <li key={id} className="rounded-md border border-line px-3 py-2">
                        <Checkbox
                          label={candidate.lane.task}
                          hint={`${
                            candidate.bytes === null ? "Size unknown" : formatBytes(candidate.bytes)
                          } · ${idleLabel(candidate.lastActivity)} · ${
                            candidate.preservation === "checkpoint"
                              ? "Uncommitted changes will be checkpointed"
                              : "Clean worktree"
                          }`}
                          checked={selected.has(id)}
                          disabled={busy || confirming}
                          onChange={(event) => toggle(id, event.target.checked)}
                        />
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>
            {confirming && (
              <div className="rounded-md border border-line bg-surface-sunken px-3 py-2">
                <p className="text-body-strong">
                  Archive {selected.size} {selected.size === 1 ? "lane" : "lanes"}?
                </p>
                <p className="mt-1 text-ink-muted">
                  Their worktrees will be removed. Branches and conversation history are kept, and
                  archived lanes can be restored from the sidebar. Any uncommitted changes will be
                  preserved as Panoptes checkpoints.
                </p>
              </div>
            )}
          </div>
        )}
      </div>
      <OverlayFooter>
        <Button
          variant="ghost"
          icon="refresh-cw"
          disabled={busy || loading || confirming}
          onClick={() => {
            setOutcome(null);
            void load();
          }}
        >
          Refresh
        </Button>
        {confirming && (
          <Button variant="secondary" disabled={busy} onClick={() => setConfirming(false)}>
            Back
          </Button>
        )}
        <Button
          variant="primary"
          className="ml-auto"
          disabled={busy || loading || selected.size === 0}
          onClick={() => {
            if (confirming) void archive();
            else {
              setOutcome(null);
              setConfirming(true);
            }
          }}
        >
          {busy
            ? "Archiving…"
            : confirming
              ? `Confirm archive (${selected.size})`
              : `Archive selected${selected.size > 0 ? ` (${selected.size})` : ""}`}
        </Button>
      </OverlayFooter>
    </Overlay>
  );
}
