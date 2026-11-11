import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { ComposerRunningIndicator } from "./ComposerRunningIndicator";

it("shows the green thinking mark and Running while the agent works", () => {
  const html = renderToStaticMarkup(<ComposerRunningIndicator status="running" />);
  expect(html).toContain('role="status"');
  expect(html).toContain("text-running");
  expect(html).toContain("px-lm-glance");
  expect(html).toContain(">Running</span>");
});

it("stays hidden for every non-running status", () => {
  for (const status of ["idle", "waiting", "failed", "done"] as const) {
    expect(renderToStaticMarkup(<ComposerRunningIndicator status={status} />)).toBe("");
  }
});
