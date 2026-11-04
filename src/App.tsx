import { useEffect, useState } from "react";
import { ChatPane } from "./components/ChatPane";
import { Sidebar } from "./components/Sidebar";
import { useStore } from "./store";

export default function App() {
  const init = useStore((s) => s.init);
  const info = useStore((s) => s.info);
  const error = useStore((s) => s.error);
  const dismissError = useStore((s) => s.dismissError);
  const [sidebarOpen, setSidebarOpen] = useState(true);

  useEffect(() => {
    void init();
  }, [init]);

  // Cmd/Ctrl+B toggles the session panel, whatever has focus.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "b" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setSidebarOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pi = info?.harnesses.find((h) => h.id === "pi");
  const banner =
    info?.errors[0] ??
    (pi && !pi.supported ? `Pi ${pi.version} is not a supported version; expect problems.` : null);

  return (
    <div className="flex h-full flex-col">
      {banner && <div className="bg-amber-100 px-4 py-1 text-xs text-amber-900">{banner}</div>}
      {error && (
        <div className="flex justify-between bg-red-100 px-4 py-1 text-xs text-red-900">
          <span>{error}</span>
          <button type="button" onClick={dismissError}>
            Dismiss
          </button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        {sidebarOpen ? (
          <Sidebar onHide={() => setSidebarOpen(false)} />
        ) : (
          <button
            type="button"
            className="flex w-9 shrink-0 flex-col items-center justify-center border-r border-neutral-200 bg-neutral-50 text-lg text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
            onClick={() => setSidebarOpen(true)}
            title="Show the session panel (Cmd/Ctrl+B)"
            aria-label="Show the session panel"
          >
            »
          </button>
        )}
        <ChatPane />
      </div>
    </div>
  );
}
