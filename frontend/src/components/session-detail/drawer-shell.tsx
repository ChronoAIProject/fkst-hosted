import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { OverlayPresence } from '@/components/ui/motion';
import { ScrollArea } from '@/components/ui/scroll-area';

/** What counts as reachable-by-Tab inside the drawer for the focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Right-anchored slide-in drawer chrome: dimmed scrim, full-height panel,
 *  Escape-to-close, and the same focus management ModalShell applies (aria-modal
 *  hides the page from assistive tech but does not move keyboard focus). The
 *  caller owns the header + body; this owns the shell + accessibility.
 *
 *  Open/unmount contract (read before reusing — the env-manager relies on it):
 *  the panel + scrim ride `OverlayPresence`, which keeps the tree mounted across
 *  an AnimatePresence exit so the drawer can slide OUT (translateX(100%)) and
 *  the scrim can fade before it disappears. That exit only plays when
 *  `DrawerShell` STAYS mounted and `open` flips to false. A caller that instead
 *  conditionally renders `{cond && <DrawerShell/>}` (e.g. the session card) still
 *  gets the slide-IN, but the close is instant — React unmounts the whole tree,
 *  including the AnimatePresence, before an exit can run. That matches the prior
 *  behavior (entry animated, exit instant), so this is a superset, not a
 *  regression. To get a real slide-OUT, keep `<DrawerShell open={open}/>` mounted
 *  and toggle `open`, calling `onClose` from the same handler that sets it false. */
export function DrawerShell({
  titleId,
  onClose,
  open = true,
  children,
}: {
  /** id of the element that labels the dialog (the drawer title). */
  titleId: string;
  onClose: () => void;
  /** Drives the slide/fade. Defaults to `true` for conditional-render callers
   *  that mount only while the drawer should be shown. Keep the shell mounted
   *  and flip this to false to get an animated exit (see the contract above). */
  open?: boolean;
  children: ReactNode;
}) {
  const panelRef = useRef<HTMLDivElement>(null);

  // On open, move focus into the drawer; on close (or unmount), hand it back to
  // the opener so keyboard users land where they left off. Keyed on `open` so
  // both contracts work: a conditional-render caller sees mount→focus-in and
  // unmount→focus-back; a keep-mounted caller sees the same on the open toggle.
  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = panelRef.current;
    if (panel) {
      const first = panel.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? panel).focus();
    }
    return () => {
      if (opener?.isConnected) opener.focus();
    };
  }, [open]);

  useEffect(() => {
    // While closed, the drawer may still be mid-exit in the DOM; don't let its
    // Escape/Tab handlers fire against a panel the user has already dismissed.
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Stop propagation so the canvas' page-level Escape (level-back) never
        // fires while the drawer is open.
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab') return;
      const panel = panelRef.current;
      if (!panel) return;
      const focusable = [...panel.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (focusable.length === 0) {
        e.preventDefault();
        panel.focus();
        return;
      }
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      const active = document.activeElement;
      const inside = active instanceof HTMLElement && panel.contains(active) && active !== panel;
      if (e.shiftKey && (!inside || active === first)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (!inside || active === last)) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [open, onClose]);

  // `role="presentation"` on the OverlayPresence panel: it already stamps a
  // hardcoded `aria-modal`, so the browser drops the presentation role and
  // treats it as a plain generic container — leaving the inner element below as
  // the single, correctly-labelled `dialog` (a second `role="dialog"` here would
  // give the page two dialogs). A frosted `bg-glass` + `backdrop-blur` panel
  // rides over the dimmed scrim for depth; the blur keeps its content legible
  // while the level-2 sidebar shows only faintly behind it. An amber→gold
  // gradient hairline (border-image) traces the left edge, catching the light as
  // the panel slides in, and a layered shadow + amber bloom seat it off the page.
  return (
    <OverlayPresence
      open={open}
      variant="drawer"
      role="presentation"
      onBackdropClick={onClose}
      className="relative flex w-full max-w-[560px] flex-col overflow-hidden border-l border-l-transparent [border-image:var(--grad-hairline-accent)_1] bg-glass backdrop-blur-glass shadow-[var(--shadow-3),var(--glow-amber),var(--highlight-top)]"
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="flex min-h-0 flex-1 flex-col outline-none"
      >
        {/* Body scroll region: the panel stays fixed-height and this scrolls
            internally, so the drawer is anchored to the viewport regardless of
            page scroll. Plain block flow (no flex) so the caller's `sticky top-0`
            header pins to THIS scroller exactly as it did on the old panel. */}
        <ScrollArea>{children}</ScrollArea>
      </div>
    </OverlayPresence>
  );
}
