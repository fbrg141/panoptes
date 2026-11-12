// The composer's text logic: @-mentions, file matching and labels. Pure, so
// it is tested without a window.

import type { ContextUsage, Draft, FileAttachment, ImageAttachment } from "./types";

/** An `@query` being typed: where its `@` is and what follows it. */
export interface MentionQuery {
  start: number;
  query: string;
}

/** The mention the caret is in, if any. An `@` starts one only at the start
 * of the text or after whitespace, so e-mail addresses don't. */
export function mentionAt(text: string, caret: number): MentionQuery | null {
  const match = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  if (!match) return null;
  return { start: caret - match[2].length - 1, query: match[2] };
}

/** Replaces the mention being typed with `@path`, returning the new text and
 * where the caret goes: after the space that follows it. */
export function insertMention(
  text: string,
  caret: number,
  mention: MentionQuery,
  path: string,
): { text: string; caret: number } {
  const rest = text.slice(caret);
  const inserted = /^\s/.test(rest) ? `@${path}` : `@${path} `;
  return {
    text: text.slice(0, mention.start) + inserted + rest,
    caret: mention.start + `@${path} `.length,
  };
}

/** Greedy subsequence score of `query` in `target`, or null if it does not
 * match. Consecutive letters and letters that start a word count most. */
function subsequence(query: string, target: string): number | null {
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const ch of query) {
    const found = target.indexOf(ch, from);
    if (found < 0) return null;
    score += 1;
    if (found === previous + 1) score += 4;
    if (found === 0 || "/._- ".includes(target[found - 1])) score += 3;
    previous = found;
    from = found + 1;
  }
  return score;
}

/** How well `path` matches a fuzzy `query`; null when it does not. Matches
 * inside the file name beat matches spread over folders, and shorter paths
 * win ties. */
export function fuzzyScore(query: string, path: string): number | null {
  const q = query.toLowerCase();
  const p = path.toLowerCase();
  const whole = subsequence(q, p);
  if (whole === null) return null;
  const name = subsequence(q, p.slice(p.lastIndexOf("/") + 1));
  const best = name === null ? whole : Math.max(whole, name + 10);
  return best - p.length / 100;
}

/** The best `limit` files for a query, best first. */
export function matchFiles(query: string, files: string[], limit = 50): string[] {
  if (!query) {
    return [...files].sort((a, b) => a.length - b.length || a.localeCompare(b)).slice(0, limit);
  }
  const scored: [string, number][] = [];
  for (const file of files) {
    const score = fuzzyScore(query, file);
    if (score !== null) scored.push([file, score]);
  }
  scored.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  return scored.slice(0, limit).map(([file]) => file);
}

export interface Segment {
  text: string;
  mention: boolean;
}

/** Splits text into plain runs and the `@path` mentions in `known`, for
 * drawing mentions as chips behind the text field. */
export function segments(text: string, known: ReadonlySet<string>): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(/(^|\s)@(\S+)/g)) {
    if (!known.has(match[2])) continue;
    const start = (match.index ?? 0) + match[1].length;
    if (start > last) out.push({ text: text.slice(last, start), mention: false });
    out.push({ text: `@${match[2]}`, mention: true });
    last = start + match[2].length + 1;
  }
  if (last < text.length) out.push({ text: text.slice(last), mention: false });
  return out;
}

/** 1200 as "1.2k", 200000 as "200k". */
export function formatTokens(n: number): string {
  const short = (value: number, unit: string) =>
    `${Number.parseFloat(value.toFixed(value < 10 ? 1 : 0))}${unit}`;
  if (n < 1000) return String(n);
  if (n < 1_000_000) return short(n / 1000, "k");
  return short(n / 1_000_000, "M");
}

/** "42k / 200k", or just the window while usage is unknown. */
export function contextLabel(usage: ContextUsage): string {
  const window = formatTokens(usage.window);
  return usage.tokens === null ? `${window} context` : `${formatTokens(usage.tokens)} / ${window}`;
}

/** The level after `current`, wrapping round. */
export function nextLevel(levels: string[], current: string | null): string | null {
  if (levels.length === 0) return null;
  const index = current === null ? -1 : levels.indexOf(current);
  return levels[(index + 1) % levels.length];
}

/** The prompt to send: images inline, other files appended as `@path`
 * mentions for the agent to read with its own tools. */
export function composePrompt(
  text: string,
  attachments: FileAttachment[],
): { text: string; images: ImageAttachment[] } {
  const images: ImageAttachment[] = [];
  const paths: string[] = [];
  for (const file of attachments) {
    if (file.mimeType && file.data) images.push({ mimeType: file.mimeType, data: file.data });
    else paths.push(`@${file.path}`);
  }
  const body = text.trim();
  return { text: [body, paths.join("\n")].filter(Boolean).join("\n\n"), images };
}

/**
 * The draft to show after a prompt failed to reach the agent: what was sent,
 * followed by anything typed since, so neither is lost.
 */
export function restoreUnsent(unsent: Draft, current: Draft | undefined): Draft {
  if (!current || (!current.text.trim() && current.attachments.length === 0)) return unsent;
  return {
    text: unsent.text.trim() ? `${unsent.text}\n${current.text}` : current.text,
    attachments: [...unsent.attachments, ...current.attachments],
    mentioned: [...new Set([...unsent.mentioned, ...current.mentioned])],
  };
}
