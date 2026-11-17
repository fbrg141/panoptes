import { type DropZone, dropZoneAt, sessionPane } from "./panes";
import { useStore } from "./store";
import { useUi } from "./ui";

/** Pointer-based session dragging: WKWebView's native file-drop handler can
 * swallow HTML5 drags, while pointer events still work inside the page. */
export interface SessionDragSnapshot {
  sessionKey: string | null;
  paneId: string | null;
  zone: DropZone | null;
}

interface PointerStart {
  button: number;
  clientX: number;
  clientY: number;
}

interface ActiveDrag {
  key: string;
  x: number;
  y: number;
  started: boolean;
}

const THRESHOLD = 6;
const IDLE: SessionDragSnapshot = { sessionKey: null, paneId: null, zone: null };
const listeners = new Set<() => void>();
let drag: ActiveDrag | null = null;
let snapshot: SessionDragSnapshot = IDLE;
let previousSelect = "";

function publish(next: SessionDragSnapshot) {
  if (
    snapshot.sessionKey === next.sessionKey &&
    snapshot.paneId === next.paneId &&
    snapshot.zone === next.zone
  ) {
    return;
  }
  snapshot = next;
  for (const listener of listeners) listener();
}

export function subscribeSessionDrag(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function sessionDragSnapshot(): SessionDragSnapshot {
  return snapshot;
}

function updateTarget(x: number, y: number) {
  const pane = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-pane]");
  if (!pane) {
    publish({ sessionKey: drag?.key ?? null, paneId: null, zone: null });
    return;
  }
  publish({
    sessionKey: drag?.key ?? null,
    paneId: pane.dataset.pane ?? null,
    zone: dropZoneAt(pane.getBoundingClientRect(), x, y),
  });
}

/** Prevent the compatibility click after a drag from reopening its source. */
function swallowCompatibilityClick() {
  const swallow = (event: MouseEvent) => {
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  window.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", swallow, true), 0);
}

/** Starts listening without changing the click's normal behaviour. A session
 * drag begins only once the pointer has moved beyond THRESHOLD. */
export function beginSessionDrag(event: PointerStart, sessionKey: string) {
  if (event.button !== 0 || drag) return;
  drag = { key: sessionKey, x: event.clientX, y: event.clientY, started: false };

  const move = (event: PointerEvent) => {
    if (!drag) return;
    if (!drag.started) {
      if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < THRESHOLD) return;
      drag.started = true;
      previousSelect = document.body.style.userSelect;
      document.body.style.userSelect = "none";
    }
    // Stop text selection and native drag gestures once this is our drag.
    event.preventDefault();
    updateTarget(event.clientX, event.clientY);
  };

  const finish = (event: PointerEvent, cancelled: boolean) => {
    if (!drag) return;
    const started = drag.started;
    const key = drag.key;
    if (started) updateTarget(event.clientX, event.clientY);
    const destination = snapshot;
    cleanup();
    if (!started) return;
    swallowCompatibilityClick();
    if (cancelled || !destination.paneId || !destination.zone) return;
    const alreadyVisible = sessionPane(useUi.getState().layout, key) !== null;
    useUi.getState().dropSession(key, destination.paneId, destination.zone);
    useStore.setState({ activeKey: key });
    if (!alreadyVisible) {
      const session = useStore.getState().sessions.find((session) => session.key === key);
      if (session) void useStore.getState().open(session);
    }
  };

  const up = (event: PointerEvent) => finish(event, false);
  const cancel = (event: PointerEvent) => finish(event, true);
  function cleanup() {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    document.body.style.userSelect = previousSelect;
    drag = null;
    publish(IDLE);
  }

  window.addEventListener("pointermove", move, { passive: false });
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
}
