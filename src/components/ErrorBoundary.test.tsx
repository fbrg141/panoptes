// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppErrorBoundary, CrashNotice, ErrorBoundary } from "./ErrorBoundary";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let explode = true;

function Bomb({ name }: { name: string }) {
  if (explode) throw new Error(`${name} broke`);
  return <p>{name} is fine</p>;
}

function Region({ name, resetKey }: { name: string; resetKey?: unknown }) {
  return (
    <ErrorBoundary
      label={name}
      resetKey={resetKey}
      fallback={({ error, reset }) => (
        <CrashNotice title={`${name} hit an error`} error={error}>
          <button type="button" onClick={reset}>
            Try again
          </button>
        </CrashNotice>
      )}
    >
      <Bomb name={name} />
    </ErrorBoundary>
  );
}

function Thrower(): never {
  throw "plain string";
}

const render = (node: React.ReactNode) => act(() => root.render(node));
const tryAgain = () =>
  act(() => {
    container.querySelector("button")?.click();
  });

beforeEach(() => {
  explode = true;
  vi.spyOn(console, "error").mockImplementation(() => {});
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

it("keeps a crash inside its region so the rest of the window still renders", async () => {
  await render(
    <>
      <Region name="pane" />
      <p>sidebar is fine</p>
    </>,
  );
  const alert = container.querySelector('[role="alert"]');
  expect(alert?.textContent).toContain("pane hit an error");
  expect(alert?.textContent).toContain("pane broke");
  expect(container.textContent).toContain("sidebar is fine");
  expect(console.error).toHaveBeenCalledWith(
    "Panoptes: the pane crashed",
    expect.any(Error),
    expect.any(String),
  );
});

it("renders the children again after Try again once the cause is gone", async () => {
  await render(<Region name="pane" />);
  explode = false;
  await tryAgain();
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(container.textContent).toContain("pane is fine");
});

it("shows the fallback again when Try again hits the same error", async () => {
  await render(<Region name="pane" />);
  await tryAgain();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("pane broke");
});

it("clears the error when its reset key changes", async () => {
  await render(<Region name="pane" resetKey="pi:a" />);
  explode = false;
  await render(<Region name="pane" resetKey="pi:a" />);
  expect(container.querySelector('[role="alert"]')).not.toBeNull();
  await render(<Region name="pane" resetKey="pi:b" />);
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(container.textContent).toContain("pane is fine");
});

it("turns a thrown non-Error into a readable message", async () => {
  await render(
    <ErrorBoundary label="x" fallback={({ error }) => <p>{error.message}</p>}>
      <Thrower />
    </ErrorBoundary>,
  );
  expect(container.textContent).toBe("plain string");
});

it("replaces a blank window with a way to recover", async () => {
  await render(
    <AppErrorBoundary>
      <Bomb name="app" />
    </AppErrorBoundary>,
  );
  expect(container.textContent).toContain("Panoptes hit an error");
  expect(container.textContent).toContain("app broke");
  const labels = [...container.querySelectorAll("button")].map((b) => b.textContent);
  expect(labels).toEqual(["Try again", "Reload window"]);
});
