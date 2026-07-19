import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { ModalShell } from './modal-shell';

/** Minimal opener → dialog harness mirroring how every modal is mounted:
 *  ModalShell is conditionally rendered, and closing unmounts it (which is
 *  exactly why ModalShell must defer the parent's onClose until its exit
 *  animation has played). */
function Harness({
  children,
  footer,
}: {
  children?: React.ReactNode;
  footer?: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button type="button" onClick={() => setOpen(true)}>
        opener
      </button>
      {open && (
        <ModalShell
          titleId="t"
          title="Test dialog"
          onClose={() => setOpen(false)}
          footer={footer}
        >
          {children ?? (
            <>
              <input aria-label="first field" />
              <button type="button">middle</button>
              <button type="button">last action</button>
            </>
          )}
        </ModalShell>
      )}
    </div>
  );
}

describe('ModalShell focus management', () => {
  it('moves focus to the first field when the dialog opens', async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole('button', { name: 'opener' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByLabelText('first field')).toHaveFocus();
  });

  it('falls back to focusing the dialog container when nothing is focusable', async () => {
    const user = userEvent.setup();
    render(
      <Harness>
        <p>static body</p>
      </Harness>
    );

    await user.click(screen.getByRole('button', { name: 'opener' }));
    expect(screen.getByRole('dialog')).toHaveFocus();
  });

  it('traps Tab inside the dialog, wrapping at both edges', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole('button', { name: 'opener' }));

    // Forward from the last element wraps to the first…
    screen.getByRole('button', { name: 'last action' }).focus();
    await user.tab();
    expect(screen.getByLabelText('first field')).toHaveFocus();

    // …and backward from the first wraps to the last. The opener behind the
    // overlay is never reached.
    await user.tab({ shift: true });
    expect(screen.getByRole('button', { name: 'last action' })).toHaveFocus();
  });

  it('restores focus to the opener once the close animation finishes', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole('button', { name: 'opener' });

    await user.click(opener);
    expect(screen.getByLabelText('first field')).toHaveFocus();

    await user.keyboard('{Escape}');
    // Close is deferred through the exit animation, so unmount + focus-restore
    // land asynchronously rather than synchronously with the keypress.
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });
});

describe('ModalShell footer slot', () => {
  it('renders the footer content when provided and keeps it in the tab cycle', async () => {
    const user = userEvent.setup();
    render(
      <Harness
        footer={
          <button type="button">submit</button>
        }
      />
    );
    await user.click(screen.getByRole('button', { name: 'opener' }));

    const submit = screen.getByRole('button', { name: 'submit' });
    expect(submit).toBeInTheDocument();

    // The footer's control is the LAST focusable, so a forward Tab from it
    // wraps back to the first field — proving the footer sits inside the trap.
    submit.focus();
    await user.tab();
    expect(screen.getByLabelText('first field')).toHaveFocus();
  });

  it('omits the footer bar when no footer prop is passed (backward compatible)', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole('button', { name: 'opener' }));

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'submit' })).not.toBeInTheDocument();
  });
});

describe('ModalShell close animation path', () => {
  /** Keeps ModalShell mounted until onClose fires so we can observe that the
   *  close is deferred (animated exit) rather than instant. */
  function SpyHarness({ onClose }: { onClose: () => void }) {
    const [open, setOpen] = useState(true);
    return open ? (
      <ModalShell
        titleId="t"
        title="Exit dialog"
        onClose={() => {
          onClose();
          setOpen(false);
        }}
      >
        <button type="button">body action</button>
      </ModalShell>
    ) : null;
  }

  it('defers onClose until the exit animation completes, then unmounts', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<SpyHarness onClose={onClose} />);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    // The exit animation runs first: onClose has NOT fired synchronously with
    // the keypress (an instant unmount would have called it already).
    expect(onClose).not.toHaveBeenCalled();

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});

describe('ModalShell under reduced motion', () => {
  const original = window.matchMedia;
  afterEach(() => {
    window.matchMedia = original;
  });

  it('still opens and closes when the user prefers reduced motion', async () => {
    // Report prefers-reduced-motion:reduce so OverlayPresence swaps instantly
    // and the exit timer collapses to a single tick.
    window.matchMedia = ((query: string) =>
      ({
        matches: query.includes('prefers-reduced-motion'),
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;

    const user = userEvent.setup();
    const onClose = vi.fn();
    function ReducedHarness() {
      const [open, setOpen] = useState(true);
      return open ? (
        <ModalShell
          titleId="t"
          title="Reduced dialog"
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        >
          <input aria-label="first field" />
        </ModalShell>
      ) : null;
    }
    render(<ReducedHarness />);

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByLabelText('first field')).toHaveFocus();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});
