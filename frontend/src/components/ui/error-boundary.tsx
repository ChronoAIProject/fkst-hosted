import { Component } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { isRouteErrorResponse, useRouteError } from 'react-router-dom';
import { useContent } from '@/i18n';

/**
 * App-shell render-error handling.
 *
 * Two entry points share one presentational fallback so a thrown render error
 * lands on a friendly, reloadable page instead of a blank white screen:
 *
 *  - `RouteErrorElement` is wired as a route `errorElement`. React Router
 *    catches errors thrown while rendering a route (or its loaders) and renders
 *    this in the erroring route's slot — so when it sits on a pathless child of
 *    the `Shell` route, the topbar/footer stay put and the fallback shows
 *    *in-shell*.
 *  - `ErrorBoundary` is a classic React error boundary (the only way to catch a
 *    descendant's render throw in React) mounted at the very top, outside the
 *    router. It is the last-resort net for anything React Router cannot reach —
 *    a provider throwing, or the router machinery itself.
 *
 * The fallback is intentionally static (no entrance animation), so there is
 * nothing for `prefers-reduced-motion` to collapse — it is already at its final
 * visual state the instant it mounts.
 */

/** Normalize whatever was thrown into a single human-readable detail line.
 *  A router error may be a `Response` (route loader), a real `Error`, or an
 *  arbitrary thrown value — cover all three rather than assuming `Error`. */
function describeError(error: unknown): string {
  if (isRouteErrorResponse(error)) {
    return `${error.status} ${error.statusText}`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  // Unknown throw shape (string, object, …): stringify defensively.
  return typeof error === 'string' ? error : 'Unknown error';
}

/**
 * The shared fallback surface. Centered, self-contained, and reduced-motion
 * safe. `role="alert"` so assistive tech announces the failure; the reload
 * control does a hard `location.reload()` — the surest way to recover from a
 * corrupted client render.
 */
export function ErrorFallbackView({ detail }: { detail?: string }) {
  const s = useContent().shell;
  return (
    <div
      role="alert"
      className="min-h-[50vh] flex flex-col items-center justify-center px-4"
    >
      {/* Elevated glass card: an accent gradient hairline + composed depth and
          amber bloom lift the failure state out of a bare centered column into
          a deliberate, in-shell surface. Purely presentational. */}
      <div className="grad-border grad-border-accent bg-glass backdrop-blur-glass w-full max-w-[560px] rounded-panel shadow-[var(--highlight-top),var(--shadow-2),var(--glow-amber)] flex flex-col items-center gap-5 text-center px-8 py-10">
        <h1 className="grad-text grad-text-fg font-display font-bold text-display-md leading-tight">
          {s.errorTitle}
        </h1>
        <p className="text-[14px] leading-relaxed text-dim max-w-[52ch]">{s.errorBody}</p>

        {detail && (
          <details className="w-full max-w-[52ch] text-left">
            <summary className="font-mono text-[11.5px] text-ghost cursor-pointer select-none hover:text-faint transition-colors">
              {s.errorDetailsSummary}
            </summary>
            <pre className="mt-2 overflow-x-auto rounded-card border border-line bg-raise px-3 py-2 font-mono text-[11.5px] text-dim whitespace-pre-wrap break-words">
              {detail}
            </pre>
          </details>
        )}

        <button
          type="button"
          onClick={() => window.location.reload()}
          className="anim-sheen font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 cursor-pointer"
        >
          {s.errorReload}
        </button>
      </div>
    </div>
  );
}

/** Route-level `errorElement`. Reads the router-captured error and renders the
 *  shared fallback. Logs it so a production render failure is traceable. */
export function RouteErrorElement() {
  const error = useRouteError();
  // Traceability: a swallowed render error is otherwise invisible in prod.
  console.error('Route render error:', error);
  return <ErrorFallbackView detail={describeError(error)} />;
}

interface ErrorBoundaryState {
  error: Error | null;
}

/**
 * Top-level React error boundary. Catches render throws from anywhere in its
 * subtree that the router's `errorElement` does not (providers, the router
 * host, the toaster). Renders the same friendly fallback rather than letting
 * React unmount the whole tree to a blank page.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // Log with the component stack so the failure can be reconstructed; the
    // fallback UI itself stays free of stack noise.
    console.error('Uncaught render error:', error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.error) {
      return <ErrorFallbackView detail={this.state.error.message} />;
    }
    return this.props.children;
  }
}
