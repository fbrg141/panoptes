import { describe, expect, it } from "vitest";
import { diffStat, editDiff } from "./diff";

const edit = (oldText: string | null, newText: string) => ({ path: "a.ts", oldText, newText });
const sign = (lines: ReturnType<typeof editDiff>) =>
  lines.map((l) => `${l.type === "add" ? "+" : l.type === "remove" ? "-" : " "}${l.text}`);

describe("editDiff", () => {
  it("keeps unchanged lines as context and aligns the rest", () => {
    expect(sign(editDiff(edit("a\nb\nc\nd", "a\nB\nc\nd\ne")))).toEqual([
      " a",
      "-b",
      "+B",
      " c",
      " d",
      "+e",
    ]);
  });

  it("shows a whole-file write as additions", () => {
    expect(sign(editDiff(edit(null, "x\ny\n")))).toEqual(["+x", "+y"]);
  });

  it("counts additions and removals across edits", () => {
    const diffs = [editDiff(edit("a", "b")), editDiff(edit("", "c\nd"))];
    expect(diffStat(diffs)).toEqual({ added: 3, removed: 1 });
  });
});
