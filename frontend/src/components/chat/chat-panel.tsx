import { useCallback, useEffect, useRef, useState } from 'react';
import { Chip } from '@/components/ui/chip';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { API_CONFIGURED } from '@/lib/env';
import { useChat } from './chat-context';
import { Composer } from './composer';
import { MessageList } from './message-list';

/** The panel's corner brackets: four static 1px L-shapes. Pure paint, no motion,
 *  so they need no reduced-motion entry. */
function PanelBrackets() {
  const accent = 'color-mix(in oklab, var(--amber) 55%, transparent)';
  const corners = [
    'left-0 top-0 border-l border-t',
    'right-0 top-0 border-r border-t',
    'bottom-0 left-0 border-b border-l',
    'bottom-0 right-0 border-b border-r',
  ];
  return (
    <>
      {corners.map((corner) => (
        <span
          key={corner}
          aria-hidden="true"
          className={`pointer-events-none absolute z-10 h-2 w-2 ${corner}`}
          style={{ borderColor: accent }}
        />
      ))}
    </>
  );
}

/** A faint scanline texture — the HUD read. Static (so reduced motion is
 *  irrelevant) and at 3% foreground, well under any text layer, so nothing it sits
 *  behind loses contrast. */
function Scanlines() {
  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 opacity-[0.35]"
      style={{
        backgroundImage:
          'repeating-linear-gradient(0deg, color-mix(in oklab, var(--fg) 3%, transparent) 0 1px, transparent 1px 3px)',
      }}
    />
  );
}

/**
 * The docked concierge panel.
 *
 * Deliberately NOT `DrawerShell`: that is a modal takeover with a scrim and a focus
 * trap, and this is a companion. The page stays interactive and visible beside it,
 * because the whole point is asking about what you are looking at.
 *
 * It follows DrawerShell's one hard lesson though — stay MOUNTED and flip `open`.
 * Conditional rendering would kill the exit animation, and would also throw away
 * the transcript's scroll position every time the panel closed.
 */
export function ChatPanel() {
  const c = useContent();
  const s = c.chat;
  const { isAuthenticated, signIn } = useAuth();
  const { open, closePanel, streaming, messages, clearTranscript } = useChat();
  const panelRef = useRef<HTMLDivElement>(null);
  const openerRef = useRef<Element | null>(null);
  // The composer's text lives here so a starter prompt can prefill it.
  const [draft, setDraft] = useState('');

  // Focus moves into the composer on open and returns to the opener on close, so a
  // keyboard user is never dropped at the top of the document.
  useEffect(() => {
    if (open) {
      openerRef.current = document.activeElement;
      panelRef.current?.querySelector<HTMLTextAreaElement>('textarea')?.focus();
      return;
    }
    const opener = openerRef.current;
    if (opener instanceof HTMLElement) opener.focus();
  }, [open]);

  // Escape closes — but ONLY from inside the panel. This is a non-modal surface, so
  // swallowing Escape globally would break the dashboard's own walk-up.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      const target = event.target;
      if (target instanceof Node && panelRef.current?.contains(target)) {
        event.stopPropagation();
        closePanel();
      }
    };
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
  }, [open, closePanel]);

  const onPickStarter = useCallback((text: string) => {
    setDraft(text);
    panelRef.current?.querySelector<HTMLTextAreaElement>('textarea')?.focus();
  }, []);

  if (!API_CONFIGURED) return null;

  return (
    <div
      ref={panelRef}
      role="complementary"
      aria-label={s.panelAria}
      data-testid="chat-panel"
      aria-hidden={!open}
      // Kept mounted; `open` drives both visibility and interactivity. `invisible`
      // rather than `hidden` so the entrance animation has something to animate.
      className={`fixed right-3 z-50 flex w-[min(480px,calc(100vw-24px))] flex-col overflow-hidden rounded-panel ${
        open ? 'anim-chat-open visible' : 'pointer-events-none invisible'
      }`}
      style={{
        // Sits below the topbar and above the pinned footer, so it never covers the
        // shell chrome the user needs to navigate away.
        top: '76px',
        bottom: '56px',
        background:
          'linear-gradient(var(--glass), var(--glass)) padding-box, var(--grad-hairline) border-box',
        border: '1px solid transparent',
        boxShadow: 'var(--shadow-2), var(--highlight-top), var(--glow-amber)',
      }}
    >
      <PanelBrackets />
      <Scanlines />
      {/* Top edge: a gradient hairline that reads as light catching the lip. */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 h-px bg-grad-hairline-accent"
      />

      <header className="relative z-[1] flex flex-none items-center gap-2 border-b border-line px-3 py-2.5 backdrop-blur-glass">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-[0.18em] text-faint">
          {s.panelTitle}
        </span>
        {/* Both states use the NEUTRAL tone: the design language reserves amber for
            the brand, never a status. The difference is carried by the text, plus a
            blinking dot while a turn is live (which reduced motion stills). */}
        <Chip tone="neutral">
          {streaming && (
            <span aria-hidden="true" className="anim-dot-blink mr-1 inline-block">
              ·
            </span>
          )}
          {streaming ? s.streaming : s.linkActive}
        </Chip>
        <span className="flex-1" aria-hidden="true" />
        {messages.length > 0 && (
          <button
            type="button"
            onClick={clearTranscript}
            aria-label={s.clearAria}
            data-testid="chat-clear"
            className="rounded-control px-2 py-1 font-mono text-[10.5px] text-faint transition-colors hover:text-fg cursor-pointer"
          >
            {s.clear}
          </button>
        )}
        <button
          type="button"
          onClick={closePanel}
          aria-label={s.closeAria}
          data-testid="chat-close"
          className="rounded-control px-2 py-1 font-mono text-[13px] leading-none text-faint transition-colors hover:text-fg cursor-pointer"
        >
          ×
        </button>
      </header>

      {isAuthenticated ? (
        <div className="relative z-[1] flex flex-1 flex-col min-h-0">
          <MessageList onPickStarter={onPickStarter} />
          <Composer value={draft} onChange={setDraft} />
        </div>
      ) : (
        // Signed out: the concierge answers with the USER's access, so there is
        // nothing useful to show until they sign in. Say that, and offer the action.
        <div
          data-testid="chat-signin-card"
          className="relative z-[1] flex flex-1 flex-col items-start justify-center gap-3 p-5"
        >
          <p className="font-display text-[15px] font-semibold text-fg">{s.signInTitle}</p>
          <p className="text-[12.5px] leading-relaxed text-dim">{s.signInBody}</p>
          <button
            type="button"
            onClick={signIn}
            className="rounded-control bg-grad-accent px-4 py-2 font-ui text-[12.5px] font-semibold text-amber-ink shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter] hover:brightness-110 cursor-pointer"
          >
            {c.auth.signIn}
          </button>
        </div>
      )}
    </div>
  );
}
