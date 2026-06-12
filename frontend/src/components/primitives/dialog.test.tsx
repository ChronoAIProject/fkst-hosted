import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Dialog, DialogTrigger, DialogContent, DialogTitle, DialogDescription } from './dialog';

describe('Dialog Primitive', () => {
  it('opens and closes via close button, Esc, and backdrop click', async () => {
    const user = userEvent.setup();
    render(
      <Dialog>
        <DialogTrigger data-testid="trigger">Open</DialogTrigger>
        <DialogContent data-testid="content">
          <DialogTitle>Dialog Title</DialogTitle>
          <DialogDescription>Description text here</DialogDescription>
          <p>Content</p>
        </DialogContent>
      </Dialog>
    );

    // Dialog is initially closed
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    // Click trigger to open
    const trigger = screen.getByTestId('trigger');
    await user.click(trigger);

    // Dialog is open
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    // Close via close button click (locks the × spec)
    const closeBtn = screen.getByRole('button', { name: /close/i });
    await user.click(closeBtn);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    // Reopen
    await user.click(trigger);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    // Close via Esc key
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    // Reopen
    await user.click(trigger);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    // Close via backdrop click
    const backdrop = screen.getByTestId('dialog-backdrop');
    await user.click(backdrop);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
