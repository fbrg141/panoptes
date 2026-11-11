import { Button, Menu, type MenuItem } from "@panoptes/design-system";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { placeMenu } from "../menuPlacement";

/**
 * The ellipsis trigger and popup shared by project, lane and session rows.
 * The popup is portalled to `body`, so no row's stacking context or overflow
 * can paint over it or clip it; `className` only positions the trigger.
 */
export function RowMenu({
  label,
  items,
  onSelect,
  className = "",
  triggerClassName = "bg-surface",
  triggerTabIndex,
  open: controlled,
  onOpenChange,
}: {
  label: string;
  items: MenuItem[];
  onSelect: (id: string) => void;
  className?: string;
  triggerClassName?: string;
  /** -1 when the row moves focus to the trigger itself (roving focus). */
  triggerTabIndex?: number;
  /** Lets the row open the menu from the keyboard (Shift+F10, the menu key). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const [uncontrolled, setUncontrolled] = useState(false);
  const open = controlled ?? uncontrolled;
  const notify = useRef(onOpenChange);
  notify.current = onOpenChange;
  const setOpen = useCallback((value: boolean) => {
    setUncontrolled(value);
    notify.current?.(value);
  }, []);
  const root = useRef<HTMLDivElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<{ left: number; top: number } | null>(null);

  // Place the popup against the trigger before it paints, and follow the
  // trigger while the sidebar scrolls or the window resizes.
  useLayoutEffect(() => {
    if (!open) {
      setAt(null);
      return;
    }
    const place = () => {
      const trigger = root.current?.querySelector("button")?.getBoundingClientRect();
      const menu = popup.current?.getBoundingClientRect();
      if (!trigger || !menu) return;
      setAt(
        placeMenu(
          trigger,
          { width: menu.width, height: menu.height },
          { width: window.innerWidth, height: window.innerHeight },
        ),
      );
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    popup.current?.querySelector<HTMLElement>("[role=menu]")?.focus();
    const close = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!root.current?.contains(target) && !popup.current?.contains(target)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open, setOpen]);

  const focusTrigger = () => root.current?.querySelector<HTMLButtonElement>("button")?.focus();
  const dismiss = () => {
    setOpen(false);
    focusTrigger();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the popup supports keyboard dismissal
    <div
      ref={root}
      className={className}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          dismiss();
        }
        if (event.key === "Tab") setOpen(false);
      }}
    >
      <Button
        variant="ghost"
        size="sm"
        icon="ellipsis-vertical"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        title={label}
        className={triggerClassName}
        tabIndex={triggerTabIndex}
        onClick={() => setOpen(!open)}
      />
      {open &&
        createPortal(
          // Transparent until placed, so it never flashes at the corner of the
          // window; still focusable, unlike `visibility: hidden`.
          <div
            ref={popup}
            className="fixed z-30"
            style={at ?? { left: 0, top: 0, opacity: 0, pointerEvents: "none" }}
          >
            <Menu
              label={label}
              items={items}
              className="max-h-80 overflow-y-auto"
              onSelect={(id) => {
                setOpen(false);
                onSelect(id);
                focusTrigger();
              }}
            />
          </div>,
          document.body,
        )}
    </div>
  );
}
