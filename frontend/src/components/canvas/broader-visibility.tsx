import { useState } from 'react';
import { useContent } from '@/i18n';

interface BroaderVisibilityBannerProps {
  /** The deployment offers the broader credential (overview.broader_oauth_available).
   *  When false the whole affordance is off and nothing renders. */
  available: boolean;
  /** A broader token is stored — the dashboard is showing ALL repos/orgs. */
  connected: boolean;
  /** Start the broader OAuth (full-page redirect). */
  onConnect: () => void;
  /** Forget the broader token (return to the installed-only view). */
  onDisconnect: () => void;
}

/**
 * The broader-visibility affordance on the canvas. Pure/presentational — it owns
 * only its local "dismissed" state; the connect/disconnect actions and the
 * connected flag are injected so it stays trivially testable. Three states:
 *   - feature off (`available === false`) → renders nothing;
 *   - offered but not connected → a dismissible "See all your repositories ·
 *     Connect" banner that redirects into the broader OAuth;
 *   - connected → a subtle "Showing all repositories · Disconnect" chip.
 * Motion is limited to `anim-row-in` (reduced-motion-safe per the design system).
 */
export function BroaderVisibilityBanner({
  available,
  connected,
  onConnect,
  onDisconnect,
}: BroaderVisibilityBannerProps) {
  const c = useContent();
  const cc = c.dashboard.canvas;
  const [dismissed, setDismissed] = useState(false);

  if (!available) return null;

  // Connected: a quiet confirmation chip with an inline disconnect — no dismiss
  // (the disconnect IS the exit) and no heavy card, so it reads as status.
  if (connected) {
    return (
      <div
        data-testid="broader-connected"
        className="anim-row-in flex-none inline-flex items-center gap-2 self-start rounded-chip border border-line bg-glass backdrop-blur-glass px-3 py-1.5 text-[12px] text-dim shadow-[var(--shadow-1)]"
      >
        <span aria-hidden="true" className="w-1.5 h-1.5 rounded-full bg-amber shadow-glow-amber" />
        <span>{cc.broaderShowingAll}</span>
        <span aria-hidden="true" className="text-ghost">
          ·
        </span>
        <button
          type="button"
          onClick={onDisconnect}
          className="font-ui font-semibold text-dim hover:text-fg transition-colors cursor-pointer"
        >
          {cc.broaderDisconnect}
        </button>
      </div>
    );
  }

  // Offered but not connected, and not dismissed this session → the invite.
  if (dismissed) return null;

  return (
    <div
      data-testid="broader-connect"
      className="anim-row-in flex-none border border-line border-l-2 border-l-amber rounded-card bg-glass backdrop-blur-glass shadow-[var(--shadow-1),var(--glow-amber)] px-4 py-3 flex items-center gap-4 flex-wrap"
    >
      <div className="min-w-0">
        <p className="font-ui font-semibold text-[13.5px] text-fg">{cc.broaderConnectTitle}</p>
        <p className="text-[12.5px] text-dim mt-0.5 max-w-[64ch]">{cc.broaderConnectHint}</p>
      </div>
      <div className="ml-auto flex-none flex items-center gap-2">
        <button
          type="button"
          onClick={onConnect}
          className="anim-sheen relative overflow-hidden font-ui font-semibold text-[12.5px] bg-grad-accent text-amber-ink rounded-control px-4 py-2 transition-[filter] hover:brightness-110 cursor-pointer shadow-[var(--shadow-1),var(--glow-amber)]"
        >
          {cc.broaderConnect}
        </button>
        <button
          type="button"
          onClick={() => setDismissed(true)}
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer"
        >
          {c.shell.toastDismiss}
        </button>
      </div>
    </div>
  );
}
