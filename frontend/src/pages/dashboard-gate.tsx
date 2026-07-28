import { useEffect, useState } from 'react';
import { Eyebrow } from '@/components/layout/eyebrow';
import { useContent } from '@/i18n';

/** The dashboard's page header. Shared by the gate, the not-configured notice, and
 *  the signed-in body, so the three views never drift apart visually. */
export function DashboardHeader({ globalAdmin = false }: { globalAdmin?: boolean }) {
  const d = useContent().dashboard;
  return (
    <header className="flex-none">
      <div className="flex items-center gap-3 flex-wrap">
        <Eyebrow>{d.eyebrow}</Eyebrow>
        {globalAdmin && (
          <span className="border border-amber/45 rounded-control bg-amber/10 px-2 py-0.5 font-mono text-[10px] font-semibold uppercase text-amber">
            {d.globalAdmin}
          </span>
        )}
      </div>
      {/* Page headline as a bright fg->dim gradient sweep (legible low end). */}
      <h1 className="grad-text grad-text-fg mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em]">
        {d.title}
      </h1>
      <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{d.lede}</p>
    </header>
  );
}

/** The cold sign-in gate: shown only to a visitor who has never signed in. An
 *  involuntary expiry keeps the dashboard body mounted with an in-place re-auth
 *  prompt instead, so the user's level and selection survive. */
export function DashboardGate({
  error,
  configured,
  onSignIn,
}: {
  /** OAuth error slug from the callback, if any. */
  error: string | null;
  configured: boolean;
  onSignIn: () => void;
}) {
  const c = useContent();
  const d = c.dashboard;
  // Locally dismissable: the auth context has no clearError (a fresh sign-in
  // clears it), so a flag keyed to `error` hides the banner until a NEW one lands.
  const [dismissed, setDismissed] = useState(false);
  useEffect(() => {
    setDismissed(false);
  }, [error]);

  return (
    <div className="flex flex-col gap-8 max-w-[960px]">
      <DashboardHeader />
      {error && !dismissed && (
        // Frosted danger notice: glass fill, red left accent + a soft red bloom.
        <div className="anim-row-in border border-line border-l-2 border-l-red rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-1),var(--glow-red)] px-4 py-3 flex items-start gap-3">
          <div className="min-w-0 flex-1">
            {/* Map the callback's real OAuth slug to specific copy; the raw slug
                stays visible (mono) so an unrecognized one is still diagnosable. */}
            <p className="text-[13px] text-dim">{d.authErrorBySlug[error] ?? d.authError}</p>
            <p className="font-mono text-[11px] text-ghost mt-1">{error}</p>
          </div>
          <button
            type="button"
            onClick={() => setDismissed(true)}
            className="flex-none font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {c.shell.toastDismiss}
          </button>
        </div>
      )}
      {/* Hero-accent sign-in card: amber->gold hairline + card depth & amber bloom. */}
      <section className="anim-row-in grad-border grad-border-accent rounded-panel p-8 max-[600px]:p-5 flex flex-col items-start gap-4 shadow-glow shadow-highlight-top">
        <h2 className="grad-text grad-text-fg font-display font-semibold text-[20px]">
          {d.signInTitle}
        </h2>
        <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.signInBody}</p>
        {configured ? (
          <button
            type="button"
            onClick={onSignIn}
            className="anim-sheen relative overflow-hidden font-ui font-semibold text-[13.5px] bg-grad-accent text-amber-ink rounded-control px-5 py-2.5 transition-[filter] hover:brightness-110 cursor-pointer shadow-[var(--shadow-2),var(--glow-amber)]"
          >
            {c.auth.signIn}
          </button>
        ) : (
          <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
        )}
      </section>
    </div>
  );
}

/** The docs-only build's notice: no API is configured, so no call is attempted. */
export function DashboardUnconfigured() {
  const d = useContent().dashboard;
  return (
    <div className="flex flex-col gap-8 max-w-[960px]">
      <DashboardHeader />
      {/* Gradient-hairline glass card frames the not-configured notice. */}
      <section className="anim-row-in grad-border rounded-panel p-8 max-[600px]:p-5 shadow-2 shadow-highlight-top">
        <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
      </section>
    </div>
  );
}
