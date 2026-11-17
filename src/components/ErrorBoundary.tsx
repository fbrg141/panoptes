import { Button, Icon } from "@panoptes/design-system";
import { Component, type ErrorInfo, type ReactNode } from "react";

type FallbackProps = { error: Error; reset: () => void };

type Props = {
  /** Names the region in the log line, e.g. "pane" or "sidebar". */
  label: string;
  children: ReactNode;
  /** Renders in place of the children after they throw. */
  fallback: (props: FallbackProps) => ReactNode;
  /** When this changes, a caught error is cleared and the children render again. */
  resetKey?: unknown;
  /** Runs when the user asks to try again, before the children render again. */
  onReset?: () => void;
};

type State = { error: Error | null; resetKey: unknown };

/**
 * Keeps a render error inside one region of the window. Without it, one
 * exception unmounts the whole React tree and leaves a blank window.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    if (Object.is(props.resetKey, state.resetKey)) return null;
    return { error: null, resetKey: props.resetKey };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error(`Panoptes: the ${this.props.label} crashed`, error, info.componentStack);
  }

  reset = () => {
    this.props.onReset?.();
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (error) return this.props.fallback({ error, reset: this.reset });
    return this.props.children;
  }
}

/** The fallback a crashed region shows: what broke, and a way back. */
export function CrashNotice({
  title,
  error,
  children,
}: {
  title: string;
  error: Error;
  children: ReactNode;
}) {
  return (
    <div
      role="alert"
      className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center"
    >
      <Icon name="circle-alert" size={20} className="text-error" />
      <div>
        <h2 className="text-body-strong">{title}</h2>
        <p className="mt-1 break-words text-ink-muted">{error.message || "Unknown error"}</p>
      </div>
      <div className="flex gap-2">{children}</div>
    </div>
  );
}

/** The last line of defence, around the whole app. */
export function AppErrorBoundary({ children }: { children: ReactNode }) {
  return (
    <ErrorBoundary
      label="window"
      fallback={({ error, reset }) => (
        <div className="flex h-full flex-col bg-bg text-ink">
          <CrashNotice title="Panoptes hit an error" error={error}>
            <Button variant="secondary" size="sm" onClick={reset}>
              Try again
            </Button>
            <Button variant="primary" size="sm" onClick={() => window.location.reload()}>
              Reload window
            </Button>
          </CrashNotice>
        </div>
      )}
    >
      {children}
    </ErrorBoundary>
  );
}
