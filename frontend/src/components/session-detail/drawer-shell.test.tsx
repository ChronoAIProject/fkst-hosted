import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { DrawerShell } from './drawer-shell';

/** Minimal labelled body: a titled heading (referenced by aria-labelledby) plus
 *  two focusable controls so the focus trap has real endpoints to wrap between. */
function Body({ titleId }: { titleId: string }) {
  return (
    <div>
      <h2 id={titleId}>Session details</h2>
      <button type="button">first</button>
      <button type="button">last</button>
    </div>
  );
}

type ShellProps = Parameters<typeof DrawerShell>[0];

function renderShell(props: Partial<ShellProps> = {}) {
  const onClose = props.onClose ?? vi.fn();
  const titleId = 'drawer-title';
  const utils = render(
    <DrawerShell titleId={titleId} onClose={onClose} open={props.open}>
      {props.children ?? <Body titleId={titleId} />}
    </DrawerShell>
  );
  return { ...utils, onClose };
}

describe('DrawerShell', () => {
  beforeEach(() => {
    // Default to full motion (matches the suite-wide matchMedia mock) so the
    // animated OverlayPresence path is what most tests exercise.
    window.matchMedia = ((query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;
  });
  afterEach(() => vi.restoreAllMocks());

  it('renders exactly one labelled modal dialog wrapping the body', () => {
    renderShell();
    // A second role="dialog" (from the OverlayPresence panel) would make this
    // throw on multiple matches — the presentation-role wrapper prevents that.
    const dialog = screen.getByRole('dialog', { name: 'Session details' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveAttribute('aria-labelledby', 'drawer-title');
    expect(screen.getByRole('button', { name: 'first' })).toBeInTheDocument();
  });

  it('does not render while open is false', () => {
    renderShell({ open: false });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('moves focus into the drawer on open', () => {
    renderShell();
    expect(screen.getByRole('button', { name: 'first' })).toHaveFocus();
  });

  it('scrolls the body internally through a bounded ScrollArea', () => {
    renderShell();
    const dialog = screen.getByRole('dialog');
    // The ScrollArea primitive is the sole internal scroller; the body lives
    // inside it, keeping the drawer viewport-anchored independent of page scroll.
    const scroller = dialog.querySelector('.overflow-y-auto');
    expect(scroller).not.toBeNull();
    expect(scroller).toContainElement(screen.getByRole('button', { name: 'first' }));
  });

  it('closes on Escape and stops the event from reaching the page', async () => {
    const user = userEvent.setup();
    const { onClose } = renderShell();
    // A page-level Escape listener must NOT also fire (the drawer swallows it).
    const pageEscape = vi.fn();
    window.addEventListener('keydown', pageEscape, false);
    try {
      await user.keyboard('{Escape}');
      expect(onClose).toHaveBeenCalledTimes(1);
      expect(pageEscape).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('keydown', pageEscape, false);
    }
  });

  it('closes when the backdrop scrim is clicked, but not the panel', () => {
    const { onClose } = renderShell();
    const dialog = screen.getByRole('dialog');
    const panel = dialog.parentElement!; // OverlayPresence panel (opaque drawer)
    const scrim = panel.parentElement!; // full-screen scrim carrying onBackdropClick

    fireEvent.click(panel);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(scrim);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('traps Tab focus within the drawer (wraps both directions)', async () => {
    const user = userEvent.setup();
    renderShell();
    const first = screen.getByRole('button', { name: 'first' });
    const last = screen.getByRole('button', { name: 'last' });

    last.focus();
    await user.tab();
    expect(first).toHaveFocus();

    first.focus();
    await user.tab({ shift: true });
    expect(last).toHaveFocus();
  });

  it('parks focus on the panel when the body has no focusable content', async () => {
    const user = userEvent.setup();
    renderShell({ children: <h2 id="drawer-title">Empty</h2> });
    const dialog = screen.getByRole('dialog');
    // No child is focusable, so opening focuses the panel itself…
    expect(dialog).toHaveFocus();
    // …and Tab keeps focus parked there rather than escaping to the page.
    await user.tab();
    expect(dialog).toHaveFocus();
  });

  it('animates out and unmounts when open flips to false (keep-mounted contract)', async () => {
    // Reduced motion makes the OverlayPresence exit resolve instantly, so the
    // unmount is deterministic — this asserts the open-toggle exit contract.
    window.matchMedia = ((query: string) =>
      ({
        matches: query.includes('reduce'),
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;

    const titleId = 'drawer-title';
    const onClose = vi.fn();
    const { rerender } = render(
      <DrawerShell titleId={titleId} onClose={onClose} open>
        <Body titleId={titleId} />
      </DrawerShell>
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    rerender(
      <DrawerShell titleId={titleId} onClose={onClose} open={false}>
        <Body titleId={titleId} />
      </DrawerShell>
    );
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});
