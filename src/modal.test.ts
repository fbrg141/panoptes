// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { tabbables, tabTarget, trapFocus } from "./modal";

function press(key: string, init: KeyboardEventInit = {}) {
  const target = document.activeElement ?? document.body;
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

/** Tab as the browser would: the trap moves focus, or focus moves on. */
function tab(backwards = false) {
  const event = press("Tab", { shiftKey: backwards });
  if (event.defaultPrevented) return;
  const all = [...document.querySelectorAll<HTMLElement>("button, input")];
  const at = all.indexOf(document.activeElement as HTMLElement);
  all[(at + (backwards ? -1 : 1) + all.length) % all.length]?.focus();
}

function setup(inner = '<input id="field" /><button id="ok">OK</button>') {
  document.body.innerHTML = `
    <button id="opener">Open</button>
    <div id="panel" tabindex="-1">${inner}</div>
    <button id="behind">Behind</button>`;
  const opener = document.getElementById("opener") as HTMLElement;
  opener.focus();
  return { opener, panel: document.getElementById("panel") as HTMLElement };
}

const byId = (id: string) => document.getElementById(id);

let release: (() => void)[] = [];
afterEach(() => {
  for (const done of release.reverse()) done();
  release = [];
  document.body.innerHTML = "";
});

function trap(panel: HTMLElement, onClose?: () => void, opener = document.activeElement) {
  const done = trapFocus(panel, () => onClose, opener);
  release.push(done);
  return () => {
    release.splice(release.indexOf(done), 1);
    done();
  };
}

describe("tab target", () => {
  const a = document.createElement("button");
  const b = document.createElement("button");
  const c = document.createElement("button");

  it("wraps from the last element to the first and back", () => {
    expect(tabTarget([a, b, c], c, false)).toBe(a);
    expect(tabTarget([a, b, c], a, true)).toBe(c);
  });

  it("lets the browser move between elements inside", () => {
    expect(tabTarget([a, b, c], a, false)).toBeNull();
    expect(tabTarget([a, b, c], c, true)).toBeNull();
  });

  it("brings focus that is outside back to the nearest edge", () => {
    expect(tabTarget([a, b, c], document.body, false)).toBe(a);
    expect(tabTarget([a, b, c], null, true)).toBe(c);
  });
});

describe("modal focus trap", () => {
  it("moves focus inside on open, to the first text field", () => {
    const { panel } = setup();
    trap(panel);
    expect(document.activeElement).toBe(byId("field"));
  });

  it("focuses the panel itself when nothing inside can take focus", () => {
    const { panel } = setup("<p>Nothing to press</p>");
    trap(panel, vi.fn());
    expect(document.activeElement).toBe(panel);
    // Tab and Escape still belong to the panel.
    tab();
    expect(document.activeElement).toBe(panel);
  });

  it("keeps Tab and Shift+Tab inside the panel", () => {
    const { panel } = setup();
    trap(panel);
    tab();
    expect(document.activeElement).toBe(byId("ok"));
    tab();
    expect(document.activeElement).toBe(byId("field"));
    tab(true);
    expect(document.activeElement).toBe(byId("ok"));
  });

  it("wraps Shift+Tab when the panel itself has focus", () => {
    const { panel } = setup();
    trap(panel);
    panel.focus();
    tab(true);
    expect(document.activeElement).toBe(byId("ok"));
  });

  it("takes the keyboard back when focus slipped to the body", () => {
    const onClose = vi.fn();
    const { panel } = setup();
    trap(panel, onClose);
    (document.activeElement as HTMLElement).blur();
    expect(document.activeElement).toBe(document.body);
    tab();
    expect(panel.contains(document.activeElement)).toBe(true);
    (document.activeElement as HTMLElement).blur();
    press("Escape");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("pulls focus that lands behind the scrim back inside", () => {
    const { panel } = setup();
    trap(panel);
    byId("behind")?.focus();
    expect(panel.contains(document.activeElement)).toBe(true);
  });

  it("closes on Escape and gives focus back to the opener", () => {
    const { opener, panel } = setup();
    const onClose = vi.fn();
    const done = trap(panel, onClose, opener);
    const event = press("Escape");
    expect(onClose).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
    done();
    expect(document.activeElement).toBe(opener);
  });

  it("ignores Escape while the panel must not be dismissed", () => {
    const { panel } = setup();
    trap(panel);
    expect(press("Escape").defaultPrevented).toBe(false);
  });

  it("leaves Escape to a control inside that already handled it", () => {
    const { panel } = setup();
    const onClose = vi.fn();
    trap(panel, onClose);
    byId("field")?.addEventListener("keydown", (event) => event.preventDefault());
    press("Escape");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("lets only the newest of two open modals answer", () => {
    const { panel } = setup();
    const under = vi.fn();
    const over = vi.fn();
    trap(panel, under);
    const top = document.createElement("div");
    top.innerHTML = '<button id="top">Top</button>';
    document.body.append(top);
    const closeTop = trap(top, over, byId("ok"));
    expect(document.activeElement).toBe(byId("top"));
    press("Escape");
    expect(over).toHaveBeenCalledOnce();
    expect(under).not.toHaveBeenCalled();
    closeTop();
    press("Escape");
    expect(under).toHaveBeenCalledOnce();
  });

  it("skips hidden and inert elements when tabbing", () => {
    const { panel } = setup(
      '<button id="a">A</button><div inert><button>Inert</button></div><button id="b" hidden>B</button>',
    );
    expect(tabbables(panel).map((element) => element.id)).toEqual(["a"]);
  });
});
