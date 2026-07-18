import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { ModalShell } from './modal-shell';

/** Minimal opener → dialog harness mirroring how every modal is mounted. */
function Harness({ children }: { children?: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button type="button" onClick={() => setOpen(true)}>
        opener
      </button>
      {open && (
        <ModalShell titleId="t" title="Test dialog" onClose={() => setOpen(false)}>
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

  it('restores focus to the opener when the dialog closes', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole('button', { name: 'opener' });

    await user.click(opener);
    expect(screen.getByLabelText('first field')).toHaveFocus();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
  });
});
