import type { FileEdit } from "./types";

export interface DiffLine {
  type: "hunk" | "context" | "add" | "remove";
  text: string;
}

/** Above this many lines on either side, the diff is not aligned line by line. */
const MAX_ALIGNED = 400;

const lines = (text: string) => (text === "" ? [] : text.replace(/\n$/, "").split("\n"));

/**
 * Line diff of one edit. Unchanged lines at the edges are kept as context,
 * and the middle is aligned on its longest common subsequence. A whole-file
 * write is all additions.
 */
export function editDiff(edit: FileEdit): DiffLine[] {
  const after = lines(edit.newText);
  if (edit.oldText === null) return after.map((text) => ({ type: "add", text }));
  const before = lines(edit.oldText);

  let start = 0;
  while (start < before.length && start < after.length && before[start] === after[start]) start++;
  let end = 0;
  while (
    end < before.length - start &&
    end < after.length - start &&
    before[before.length - 1 - end] === after[after.length - 1 - end]
  )
    end++;

  const head = before.slice(0, start).map((text): DiffLine => ({ type: "context", text }));
  const tail = before
    .slice(before.length - end)
    .map((text): DiffLine => ({ type: "context", text }));
  return [
    ...head,
    ...align(before.slice(start, before.length - end), after.slice(start, after.length - end)),
    ...tail,
  ];
}

function align(a: string[], b: string[]): DiffLine[] {
  const removed = a.map((text): DiffLine => ({ type: "remove", text }));
  const added = b.map((text): DiffLine => ({ type: "add", text }));
  if (a.length === 0 || b.length === 0 || a.length > MAX_ALIGNED || b.length > MAX_ALIGNED) {
    return [...removed, ...added];
  }
  // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..].
  const lcs = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ type: "context", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push(removed[i++]);
    } else {
      out.push(added[j++]);
    }
  }
  return [...out, ...removed.slice(i), ...added.slice(j)];
}

/** Lines added and removed across a call's edits, for its row. */
export function diffStat(diffs: DiffLine[][]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diffs.flat()) {
    if (line.type === "add") added++;
    if (line.type === "remove") removed++;
  }
  return { added, removed };
}
