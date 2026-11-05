import { Button, Input, StatusPill } from "@panoptes/design-system";
import { useEffect, useState } from "react";
import { useStore } from "../store";
import type { Question } from "../types";

/** A question from the harness, shown until answered or until it expires. */
export function QuestionCard({ question }: { question: Question }) {
  const answer = useStore((s) => s.answer);
  const [text, setText] = useState(
    question.kind.type === "editor" ? (question.kind.prefill ?? "") : "",
  );
  const [expired, setExpired] = useState(
    question.expiresAt !== null && question.expiresAt <= Date.now(),
  );

  useEffect(() => {
    if (question.expiresAt === null) return;
    const timer = setTimeout(() => setExpired(true), Math.max(0, question.expiresAt - Date.now()));
    return () => clearTimeout(timer);
  }, [question.expiresAt]);

  if (expired) return null;

  const dismiss = () => void answer(question.id, { type: "cancelled" });
  const submitText = () => void answer(question.id, { type: "value", value: text });
  const { kind } = question;

  return (
    <section
      aria-label={question.title}
      className="mx-auto mb-3 w-full max-w-3xl rounded-md border border-line bg-surface-raised p-4 shadow-[var(--shadow-pop)]"
    >
      <div className="flex items-center gap-2">
        <StatusPill status="waiting" />
        <h2 className="text-heading">{question.title}</h2>
      </div>
      {question.message && (
        <p className="mt-2 whitespace-pre-wrap text-ink-muted">{question.message}</p>
      )}
      {kind.type === "input" && (
        <Input
          className="mt-3"
          aria-label={question.title}
          placeholder={kind.placeholder ?? ""}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submitText()}
        />
      )}
      {kind.type === "editor" && (
        <textarea
          aria-label={question.title}
          className="text-code mt-3 min-h-24 w-full rounded-sm border border-line-strong bg-surface-sunken p-2 text-ink focus:border-focus focus:outline-2 focus:outline-offset-1 focus:outline-focus"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
      )}
      <div className="mt-3 flex flex-wrap items-center justify-end gap-2 border-t border-line pt-3">
        <Button variant="ghost" onClick={dismiss}>
          Dismiss
        </Button>
        {kind.type === "select" &&
          kind.options.map((option) => (
            <Button
              key={option}
              onClick={() => void answer(question.id, { type: "value", value: option })}
            >
              {option}
            </Button>
          ))}
        {kind.type === "confirm" && (
          <>
            <Button
              onClick={() => void answer(question.id, { type: "confirmed", confirmed: false })}
            >
              Decline
            </Button>
            <Button
              variant="primary"
              onClick={() => void answer(question.id, { type: "confirmed", confirmed: true })}
            >
              Approve
            </Button>
          </>
        )}
        {(kind.type === "input" || kind.type === "editor") && (
          <Button variant="primary" onClick={submitText}>
            Submit
          </Button>
        )}
      </div>
    </section>
  );
}
