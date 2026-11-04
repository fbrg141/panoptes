import { useEffect } from "react";
import { ChatPane } from "./components/ChatPane";
import { Sidebar } from "./components/Sidebar";
import { useStore } from "./store";

export default function App() {
  const init = useStore((s) => s.init);
  const info = useStore((s) => s.info);
  const error = useStore((s) => s.error);
  const dismissError = useStore((s) => s.dismissError);

  useEffect(() => {
    void init();
  }, [init]);

  const pi = info?.harnesses.find((h) => h.id === "pi");
  const banner = info?.errors[0] ?? (pi && !pi.supported ? `Pi ${pi.version} is not a supported version; expect problems.` : null);

  return (
    <div className="flex h-full flex-col">
      {banner && <div className="bg-amber-100 px-4 py-1 text-xs text-amber-900">{banner}</div>}
      {error && (
        <div className="flex justify-between bg-red-100 px-4 py-1 text-xs text-red-900">
          <span>{error}</span>
          <button onClick={dismissError}>Dismiss</button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <ChatPane />
      </div>
    </div>
  );
}
