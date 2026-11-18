import { useMemo } from "react";
import { useStore } from "../store";
import type { Message } from "../types";
import { sessionUsageDetail, sessionUsageLabel, sumUsage } from "../usage";

const NO_MESSAGES: Message[] = [];

/** What the session's model calls have cost so far, beside the composer. */
export function SessionUsage({ sessionKey }: { sessionKey: string }) {
  const messages = useStore((s) => s.conversations[sessionKey]?.messages ?? NO_MESSAGES);
  const total = useMemo(() => sumUsage(messages), [messages]);
  if (!total) return null;
  return (
    <span
      className="pr-3 text-code-sm tabular-nums text-ink-muted"
      title={sessionUsageDetail(total)}
    >
      {sessionUsageLabel(total)}
    </span>
  );
}
