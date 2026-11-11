import { LoopMark } from "@panoptes/design-system";
import type { SessionStatus } from "../types";

/** A quiet running cue immediately above the composer. */
export function ComposerRunningIndicator({ status }: { status: SessionStatus }) {
  if (status !== "running") return null;
  return (
    <div
      role="status"
      className="mx-auto flex w-full max-w-3xl items-center gap-2 pb-1 text-running"
    >
      <span aria-hidden="true" className="flex">
        <LoopMark size={16} state="thinking" tone="current" title="Running" />
      </span>
      <span>Running</span>
    </div>
  );
}
