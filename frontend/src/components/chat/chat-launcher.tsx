import { useContent } from '@/i18n';
import { API_CONFIGURED } from '@/lib/env';
import { useChat } from './chat-context';

/**
 * The floating entry point: a glass pill with a live dot, bottom-right.
 *
 * Two placement facts are load-bearing:
 *
 * - `z-[55]` sits BELOW the toaster's `z-[60]`, so a transient toast is never
 *   hidden behind the launcher.
 * - It is absent entirely on a docs-only build, because a chat surface with no
 *   backend can only disappoint.
 */
export function ChatLauncher() {
  const s = useContent().chat;
  const { open, toggle } = useChat();

  if (!API_CONFIGURED) return null;

  return (
    <button
      type="button"
      onClick={toggle}
      aria-expanded={open}
      aria-label={open ? s.launcherCloseAria : s.launcherAria}
      data-testid="chat-launcher"
      // The two-layer padding/border-box background is the ModalShell recipe: it
      // keeps the fill translucent (glass) so the hairline catches light without
      // the opaque --raise that `.grad-border` would impose and the blur would lose.
      style={{
        background:
          'linear-gradient(var(--glass), var(--glass)) padding-box, var(--grad-hairline) border-box',
      }}
      className="hover-lift fixed bottom-6 right-6 z-[55] flex items-center gap-2 rounded-pill border border-transparent px-4 py-2.5 font-mono text-[11px] font-semibold uppercase tracking-[0.14em] text-dim backdrop-blur-glass shadow-[var(--shadow-2),var(--glow-amber)] transition-colors hover:text-fg cursor-pointer"
    >
      {/* Decorative liveness cue; the label carries the meaning. */}
      <span
        aria-hidden="true"
        className="anim-dot-blink h-1.5 w-1.5 rounded-full bg-amber shadow-glow-amber"
      />
      {s.launcherLabel}
    </button>
  );
}
