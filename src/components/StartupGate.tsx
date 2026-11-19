import { Button, Icon, LoopMark } from "@panoptes/design-system";
import { type ReactNode, useEffect, useState } from "react";
import type { StartupStatus } from "../store";

const LOADER_DELAY = 150;

/** Keeps brief startup loads quiet, but never leaves a failure on a spinner. */
export function StartupGate({
  status,
  error,
  onRetry,
  children,
}: {
  status: StartupStatus;
  error: string | null;
  onRetry: () => void;
  children: ReactNode;
}) {
  const [showLoader, setShowLoader] = useState(false);

  useEffect(() => {
    if (status !== "loading") {
      setShowLoader(false);
      return;
    }
    const timer = setTimeout(() => setShowLoader(true), LOADER_DELAY);
    return () => clearTimeout(timer);
  }, [status]);

  if (status === "ready") return children;

  return (
    <div className="flex h-full bg-bg text-ink">
      <aside className="flex w-80 shrink-0 flex-col border-r border-line bg-surface">
        <header className="flex h-[var(--titlebar)] items-center gap-2 border-b border-line px-3">
          <LoopMark size={20} />
          <span className="text-wordmark">Panoptes</span>
        </header>
        <div className="p-4">{status === "loading" && showLoader && <Loading />}</div>
      </aside>
      <main className="flex flex-1 items-center justify-center px-6">
        {status === "error" ? (
          <div className="flex max-w-md flex-col items-center gap-4 text-center" role="alert">
            <Icon name="circle-alert" size={24} className="text-error" />
            <div>
              <h1 className="text-title">Could not load Panoptes</h1>
              <p className="mt-2 text-ink-muted">{error ?? "Unknown error"}</p>
            </div>
            <Button variant="secondary" icon="refresh-cw" onClick={onRetry}>
              Try again
            </Button>
          </div>
        ) : (
          showLoader && <Loading />
        )}
      </main>
    </div>
  );
}

function Loading() {
  return (
    <div
      className="flex items-center justify-center gap-3 text-ink-muted"
      role="status"
      aria-label="Loading projects and sessions"
    >
      <LoopMark size={20} state="loading" tone="current" />
      <span>Loading projects and sessions…</span>
    </div>
  );
}
