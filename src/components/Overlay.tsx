import { Icon } from "@panoptes/design-system";
import { useEffect, useRef } from "react";

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * The window's modal layer: a scrim, a panel near the top like a command
 * palette, and the keyboard behaviour a modal owes. Escape and a click on the
 * scrim close it, Tab stays inside, and focus goes back where it was when it
 * closes. The panel's content decides its own header and footer.
 */
export function Overlay({
  label,
  width,
  onClose,
  children,
}: {
  /** The panel's accessible name. */
  label: string;
  width: number;
  /** Absent while the panel must not be dismissed, as during a submit. */
  onClose?: () => void;
  children: React.ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);

  // Remember what had focus, and give it back on close.
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    if (!panel.current?.contains(document.activeElement)) {
      panel.current?.querySelector<HTMLElement>("[autofocus], [data-autofocus]")?.focus();
    }
    return () => {
      if (before?.isConnected) before.focus();
    };
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && onClose) {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key !== "Tab" || !panel.current) return;
    const items = [...panel.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the scrim closes on click; Escape is handled inside
    <div
      className="overlay-scrim fixed inset-0 z-50 flex items-start justify-center bg-scrim px-6 pt-[12vh]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose?.();
      }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal
        aria-label={label}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        className="overlay-panel flex max-h-[76vh] max-w-full flex-col rounded-lg bg-surface-raised text-ink shadow-[var(--shadow-pop)] focus:outline-none"
        style={{ width }}
      >
        {children}
      </div>
    </div>
  );
}

/** A panel's top line: what it is, and where it acts. */
export function OverlayHeader({
  children,
  onClose,
}: {
  children: React.ReactNode;
  onClose?: () => void;
}) {
  return (
    <div className="flex h-11 flex-none items-center gap-2 px-4">
      <div className="flex min-w-0 flex-1 items-center gap-1.5">{children}</div>
      {onClose && (
        <button
          type="button"
          aria-label="Close"
          title="Close (Esc)"
          onClick={onClose}
          className="-mr-1.5 flex size-[var(--control-sm)] flex-none items-center justify-center rounded-sm text-ink-muted hover:bg-selection hover:text-ink"
        >
          <Icon name="x" size={16} />
        </button>
      )}
    </div>
  );
}

/** A panel's bottom line: options on the left, the one action on the right. */
export function OverlayFooter({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-12 flex-none flex-wrap items-center gap-2 border-t border-line px-3 py-2">
      {children}
    </div>
  );
}

/**
 * An on/off option in a panel's footer, styled as a chip beside the pickers,
 * so the footer reads as one row of choices about what happens next.
 */
export function OptionToggle({
  label,
  title,
  pressed,
  disabled,
  onChange,
}: {
  label: string;
  title?: string;
  pressed: boolean;
  disabled?: boolean;
  onChange: (pressed: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={pressed}
      title={title}
      disabled={disabled}
      onClick={() => onChange(!pressed)}
      className="picker-trigger is-chip flex h-[var(--control-sm)] flex-none items-center gap-1.5 rounded-sm px-2 text-ink-muted transition-colors duration-[var(--duration-fast)] disabled:text-ink-faint"
    >
      <span
        aria-hidden
        className={`flex size-3.5 items-center justify-center rounded-xs border ${
          pressed ? "border-focus bg-focus text-on-focus" : "border-line-strong"
        }`}
      >
        {pressed && <Icon name="check" size={12} />}
      </span>
      {label}
    </button>
  );
}
