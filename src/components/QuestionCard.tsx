import { useEffect, useState } from "react";
import { useStore } from "../store";
import type { Question } from "../types";

const buttonClass =
  "rounded border border-neutral-300 px-3 py-1 text-sm hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800";

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

  const cancel = () => void answer(question.id, { type: "cancelled" });
  const submitText = () => void answer(question.id, { type: "value", value: text });

  return (
    <div className="mx-auto mb-2 w-full max-w-3xl rounded border border-amber-300 bg-amber-50 p-3 text-sm dark:border-amber-800 dark:bg-amber-950/40">
      <div className="font-medium">{question.title}</div>
      {question.message && (
        <div className="mt-1 whitespace-pre-wrap text-neutral-600 dark:text-neutral-400">
          {question.message}
        </div>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        {question.kind.type === "select" &&
          question.kind.options.map((option) => (
            <button
              type="button"
              key={option}
              className={buttonClass}
              onClick={() => void answer(question.id, { type: "value", value: option })}
            >
              {option}
            </button>
          ))}
        {question.kind.type === "confirm" && (
          <>
            <button
              type="button"
              className={buttonClass}
              onClick={() => void answer(question.id, { type: "confirmed", confirmed: true })}
            >
              Yes
            </button>
            <button
              type="button"
              className={buttonClass}
              onClick={() => void answer(question.id, { type: "confirmed", confirmed: false })}
            >
              No
            </button>
          </>
        )}
        {question.kind.type === "input" && (
          <input
            className="min-w-0 flex-1 rounded border border-neutral-300 bg-white px-2 py-1 dark:border-neutral-700 dark:bg-neutral-950"
            placeholder={question.kind.placeholder ?? ""}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submitText()}
          />
        )}
        {question.kind.type === "editor" && (
          <textarea
            className="min-h-24 w-full rounded border border-neutral-300 bg-white px-2 py-1 font-mono text-xs dark:border-neutral-700 dark:bg-neutral-950"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
        )}
        {(question.kind.type === "input" || question.kind.type === "editor") && (
          <button type="button" className={buttonClass} onClick={submitText}>
            Submit
          </button>
        )}
        <button type="button" className={`${buttonClass} ml-auto`} onClick={cancel}>
          Dismiss
        </button>
      </div>
    </div>
  );
}
