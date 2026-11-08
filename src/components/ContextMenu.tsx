import { Menu, type MenuItem } from "@panoptes/design-system";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

/**
 * Opens a menu at the pointer when the content it wraps is right-clicked.
 * Escape and a click outside dismiss it; the menu itself takes arrows and Enter.
 */
export function ContextMenu({
  label,
  items,
  onSelect,
  className = "",
  children,
}: {
  label: string;
  items: MenuItem[];
  onSelect: (id: string) => void;
  className?: string;
  children: ReactNode;
}) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!at) return;
    root.current?.querySelector<HTMLElement>("[role=menu]")?.focus();
    const close = (event: Event) => {
      if (event.target instanceof Element && !event.target.closest("[role=menu]")) setAt(null);
    };
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setAt(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", onEscape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", onEscape);
    };
  }, [at]);

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the popup supports keyboard dismissal
    <div
      ref={root}
      className={className}
      onContextMenu={(event) => {
        event.preventDefault();
        setAt({ x: event.clientX, y: event.clientY });
      }}
    >
      {children}
      {at &&
        createPortal(
          <div className="fixed z-30 rounded-md" style={{ left: at.x, top: at.y }}>
            <Menu
              label={label}
              items={items}
              onSelect={(id) => {
                setAt(null);
                onSelect(id);
              }}
            />
          </div>,
          document.body,
        )}
    </div>
  );
}
