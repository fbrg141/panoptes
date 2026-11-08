import { Button, Menu, type MenuItem } from "@panoptes/design-system";
import { useEffect, useRef, useState } from "react";

/** The ellipsis trigger and popup shared by project, lane and session rows. */
export function RowMenu({
  label,
  items,
  onSelect,
  className = "",
}: {
  label: string;
  items: MenuItem[];
  onSelect: (id: string) => void;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLElement>("[role=menu]")?.focus();
    const close = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const focusTrigger = () => root.current?.querySelector<HTMLButtonElement>("button")?.focus();
  const dismiss = () => {
    setOpen(false);
    focusTrigger();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the popup supports keyboard dismissal
    <div
      ref={root}
      className={`relative ${className}`}
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
        className="bg-surface"
        onClick={() => setOpen((shown) => !shown)}
      />
      {open && (
        <div className="absolute top-full right-0 z-30 mt-1 max-h-80 overflow-y-auto rounded-md">
          <Menu
            label={label}
            items={items}
            onSelect={(id) => {
              setOpen(false);
              onSelect(id);
              focusTrigger();
            }}
          />
        </div>
      )}
    </div>
  );
}
