/**
 * The keyboard contract every modal panel keeps: focus moves inside when it
 * opens, Tab and Shift+Tab cycle inside it, focus that lands behind it is
 * pulled back, Escape closes it, and focus returns to the opener on close.
 *
 * The listeners sit on the document rather than on the panel, so the panel
 * keeps the keyboard even when focus has slipped out of it (to the body, or
 * to the window behind the scrim). A key a control inside already handled,
 * by calling `preventDefault` or `stopPropagation` (a picker backing out on
 * Escape, the composer closing its mention menu), is left alone. Only the
 * newest open modal answers, so a modal over another closes on its own.
 */

const FOCUSABLE = [
  "a[href]",
  "button:not(:disabled)",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

/** Where focus goes when a panel opens and nothing inside has it yet. */
const FIRST_FOCUS = [
  "[autofocus]",
  "[data-autofocus]",
  'input:not(:disabled):not([type="checkbox"]):not([type="radio"])',
  "textarea:not(:disabled)",
].join(", ");

const open: HTMLElement[] = [];

/** The panel's tabbable elements, in document order. */
export function tabbables(panel: HTMLElement): HTMLElement[] {
  return [...panel.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) => !element.closest("[inert], [hidden]"),
  );
}

/**
 * Where a Tab press should land, or null to let the browser move focus
 * between two elements that are both inside the panel.
 */
export function tabTarget(
  items: HTMLElement[],
  active: Element | null,
  backwards: boolean,
): HTMLElement | null {
  if (items.length === 0) return null;
  const at = active ? items.indexOf(active as HTMLElement) : -1;
  const first = items[0];
  const last = items[items.length - 1];
  if (at === -1) return backwards ? last : first;
  if (backwards && at === 0) return last;
  if (!backwards && at === items.length - 1) return first;
  return null;
}

/**
 * Hold the keyboard inside `panel` until the returned function is called.
 * `onClose` is read on every Escape, so it can change while the panel is
 * open; when it returns undefined (as during a submit) Escape does nothing.
 * `opener` is what had focus before the panel opened; focus goes back to it.
 */
export function trapFocus(
  panel: HTMLElement,
  onClose: () => (() => void) | undefined,
  opener: Element | null,
): () => void {
  open.push(panel);
  const newest = () => open[open.length - 1] === panel;

  const enter = () => {
    const target = panel.querySelector<HTMLElement>(FIRST_FOCUS) ?? tabbables(panel)[0] ?? panel;
    target.focus();
  };
  if (!panel.contains(document.activeElement)) enter();

  const onKeyDown = (event: KeyboardEvent) => {
    if (!newest() || event.defaultPrevented || event.isComposing) return;
    if (event.key === "Escape") {
      const close = onClose();
      if (!close) return;
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== "Tab") return;
    const items = tabbables(panel);
    const next =
      items.length === 0 ? panel : tabTarget(items, document.activeElement, event.shiftKey);
    if (!next) return;
    event.preventDefault();
    next.focus();
  };

  const onFocusIn = (event: FocusEvent) => {
    if (!newest() || !(event.target instanceof Node) || panel.contains(event.target)) return;
    enter();
  };

  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("focusin", onFocusIn);
  return () => {
    document.removeEventListener("keydown", onKeyDown);
    document.removeEventListener("focusin", onFocusIn);
    open.splice(open.indexOf(panel), 1);
    const back = opener instanceof HTMLElement && !panel.contains(opener) ? opener : null;
    if (back?.isConnected) back.focus();
  };
}
