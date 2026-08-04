import { useContent } from '@/i18n';

/**
 * The two pre-data states of `/operations`.
 *
 * Both are deliberately plain. This is not a marketing surface and never
 * acquires one: an unauthenticated visitor gets one sentence about what the
 * route is for and a sign-in button, and a build with no API base gets the one
 * fact that explains why nothing will load.
 */
export function OperationsGate({
  error,
  configured,
  onSignIn,
}: {
  /** OAuth error slug from the login callback, if any. */
  error: string | null;
  configured: boolean;
  onSignIn: () => void;
}) {
  const c = useContent();
  const t = c.operations;
  return (
    <div className="h-full flex items-center justify-center px-6">
      <section className="grad-border grad-border-accent rounded-panel p-8 max-[600px]:p-5 flex flex-col items-start gap-4 shadow-glow shadow-highlight-top max-w-[56ch]">
        <h1 className="font-display font-semibold text-[20px] text-fg">{t.gateTitle}</h1>
        <p className="text-[14px] leading-relaxed text-dim">{t.gateBody}</p>
        {error && (
          // The raw slug stays visible so an unrecognized one is diagnosable;
          // the localized sentence above it carries the meaning.
          <p className="font-mono text-[11px] text-ghost">{error}</p>
        )}
        {configured ? (
          <button
            type="button"
            onClick={onSignIn}
            className="anim-sheen relative overflow-hidden font-ui font-semibold text-[13px] bg-grad-accent text-amber-ink rounded-control px-4 py-2 shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter] hover:brightness-110 cursor-pointer"
          >
            {t.gateAction}
          </button>
        ) : (
          <p className="font-mono text-[11.5px] text-warn">{t.unconfiguredBody}</p>
        )}
      </section>
    </div>
  );
}

/** No API base URL is configured for this build, so no request can be made. */
export function OperationsUnconfigured() {
  const t = useContent().operations;
  return (
    <div className="h-full flex items-center justify-center px-6">
      <section className="grad-border rounded-panel p-8 flex flex-col items-start gap-3 shadow-2 max-w-[56ch]">
        <h1 className="font-display font-semibold text-[18px] text-fg">{t.unconfiguredTitle}</h1>
        <p className="text-[13.5px] leading-relaxed text-dim">{t.unconfiguredBody}</p>
      </section>
    </div>
  );
}
