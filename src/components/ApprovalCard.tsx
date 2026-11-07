import { Button, StatusPill } from "@panoptes/design-system";
import { useEffect, useState } from "react";
import { useStore } from "../store";
import type { Approval } from "../types";

/**
 * Allow or reject a tool call. Shown at the call it is about, until it is
 * decided or expires.
 */
export function ApprovalCard({
  approval,
  pill = true,
}: {
  approval: Approval;
  /** Off where the call's own row already says Waiting. */
  pill?: boolean;
}) {
  const answer = useStore((s) => s.answer);
  const [expired, setExpired] = useState(
    approval.expiresAt !== null && approval.expiresAt <= Date.now(),
  );

  useEffect(() => {
    if (approval.expiresAt === null) return;
    const timer = setTimeout(() => setExpired(true), Math.max(0, approval.expiresAt - Date.now()));
    return () => clearTimeout(timer);
  }, [approval.expiresAt]);

  if (expired) return null;

  const decide = (allow: boolean) => void answer(approval.id, { type: "decision", allow });
  return (
    <section
      aria-label={approval.title}
      className="rounded-md border border-line bg-surface-raised p-3 shadow-[var(--shadow-pop)]"
    >
      <div className="flex items-start gap-2">
        {pill && <StatusPill status="waiting" />}
        <h2 className="min-w-0 flex-1 whitespace-pre-wrap break-words">{approval.title}</h2>
      </div>
      {approval.message && (
        <p className="mt-2 whitespace-pre-wrap text-ink-muted">{approval.message}</p>
      )}
      <div className="mt-3 flex items-center justify-end gap-2">
        <Button onClick={() => decide(false)}>Reject</Button>
        <Button variant="primary" onClick={() => decide(true)}>
          Allow
        </Button>
      </div>
    </section>
  );
}
