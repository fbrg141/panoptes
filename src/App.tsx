import { Button, Icon } from "@panoptes/design-system";
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
  const warning =
    info?.errors[0] ??
    (pi && !pi.supported ? `Pi ${pi.version} is not a supported version. Expect problems.` : null);

  return (
    <div className="flex h-full flex-col bg-bg text-ink">
      {warning && (
        <div className="flex items-center gap-2 border-b border-line bg-waiting-soft px-4 py-1 text-waiting">
          <Icon name="circle-alert" size={14} />
          <span>{warning}</span>
        </div>
      )}
      {error && (
        <div
          role="alert"
          className="flex items-center gap-2 border-b border-line bg-error-soft px-4 py-1 text-error"
        >
          <Icon name="circle-alert" size={14} />
          <span className="flex-1">{error}</span>
          <Button variant="ghost" size="sm" onClick={dismissError}>
            Dismiss
          </Button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <ChatPane />
      </div>
    </div>
  );
}
