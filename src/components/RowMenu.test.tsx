// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RowMenu } from "./RowMenu";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it("opens the popup outside the row and selects an action", () => {
  const onSelect = vi.fn();
  act(() => {
    root.render(
      <RowMenu
        label="Actions for lane fix-login"
        className="absolute -translate-y-1/2"
        items={[{ id: "archive-lane", label: "Archive lane" }]}
        onSelect={onSelect}
      />,
    );
  });
  const trigger = container.querySelector<HTMLButtonElement>("button");
  if (!trigger) throw new Error("The row trigger did not render");
  trigger.getBoundingClientRect = () =>
    ({
      x: 100,
      y: 40,
      width: 24,
      height: 24,
      top: 40,
      right: 124,
      bottom: 64,
      left: 100,
      toJSON: () => ({}),
    }) as DOMRect;

  act(() => trigger.dispatchEvent(new MouseEvent("click", { bubbles: true })));

  const menu = document.querySelector<HTMLElement>("[role=menu]");
  const popup = menu?.parentElement;
  expect(menu).not.toBeNull();
  if (!menu) throw new Error("The menu did not open");
  expect(container.contains(menu)).toBe(false);
  expect(popup?.parentElement).toBe(document.body);
  expect(popup?.style.left).toBe("124px");
  expect(popup?.style.top).toBe("68px");

  const archive = menu.querySelector<HTMLElement>("[role=menuitem]");
  expect(archive?.textContent).toBe("Archive lane");
  if (!archive) throw new Error("The archive action did not render");
  act(() => archive.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  expect(onSelect).toHaveBeenCalledWith("archive-lane");
  expect(document.activeElement).toBe(trigger);
});
