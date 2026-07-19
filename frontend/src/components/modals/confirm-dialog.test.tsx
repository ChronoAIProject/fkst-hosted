import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ToastProvider, Toaster } from '@/components/ui/toast';
import type { MutationResult } from '@/lib/api/canvas';
import { ConfirmDialog } from './confirm-dialog';

const LABELS = {
  title: 'Stop session widgets?',
  body: 'This closes trigger issue #7.',
  confirmLabel: 'Stop session',
  pendingLabel: 'Stopping…',
  cancelLabel: 'Cancel',
  fallbackError: 'Could not stop the session. Please try again.',
};

/** Render the dialog inside a live toast surface so a raised success notice is
 *  actually drawn and observable. Every knob has a spy default the test can read
 *  back or override. */
function renderDialog(
  over: {
    action?: () => Promise<MutationResult<unknown>>;
    successMessage?: string;
    onClose?: () => void;
    onDone?: () => void;
  } = {}
) {
  const action = over.action ?? vi.fn(async () => ({ ok: true, data: null }) as MutationResult<unknown>);
  const onClose = over.onClose ?? vi.fn();
  const onDone = over.onDone ?? vi.fn();
  render(
    <ToastProvider>
      <ConfirmDialog
        {...LABELS}
        action={action}
        successMessage={over.successMessage}
        onClose={onClose}
        onDone={onDone}
      />
      <Toaster />
    </ToastProvider>
  );
  return { action, onClose, onDone };
}

describe('ConfirmDialog', () => {
  it('runs the action, raises the success toast, then calls onDone', async () => {
    const user = userEvent.setup();
    const { action, onDone } = renderDialog({ successMessage: 'Session stopped' });

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    expect(action).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    // The toast text is drawn by the live <Toaster>.
    expect(await screen.findByText('Session stopped')).toBeInTheDocument();
  });

  it('stays silent on success when no successMessage is supplied', async () => {
    const user = userEvent.setup();
    const { onDone } = renderDialog();

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    // Nothing to toast: the caller (e.g. env-detail) handles its own notice.
    expect(screen.queryByText('Session stopped')).not.toBeInTheDocument();
  });

  it('shows the envelope message with an animated entrance and keeps the dialog open on failure', async () => {
    const user = userEvent.setup();
    const action = vi.fn(async () => ({ ok: false, message: 'issue already closed' }) as MutationResult<unknown>);
    const { onDone } = renderDialog({ action, successMessage: 'Session stopped' });

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    const note = await screen.findByText('issue already closed');
    // The error rides `.anim-notice-in` so it slides in rather than popping.
    expect(note.closest('.anim-notice-in')).not.toBeNull();
    expect(onDone).not.toHaveBeenCalled();
    // No success toast on a failed mutation.
    expect(screen.queryByText('Session stopped')).not.toBeInTheDocument();
    // The confirm button is still present — the user can retry in place.
    expect(screen.getByRole('button', { name: LABELS.confirmLabel })).toBeInTheDocument();
  });

  it('falls back to the generic error when the mutation message is null', async () => {
    const user = userEvent.setup();
    const action = vi.fn(async () => ({ ok: false, message: null }) as MutationResult<unknown>);
    renderDialog({ action });

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    expect(await screen.findByText(LABELS.fallbackError)).toBeInTheDocument();
  });

  it('falls back to the generic error when the action throws', async () => {
    const user = userEvent.setup();
    const action = vi.fn(async () => {
      throw new Error('network down');
    });
    renderDialog({ action });

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    expect(await screen.findByText(LABELS.fallbackError)).toBeInTheDocument();
  });

  it('clears the prior error and succeeds on retry', async () => {
    const user = userEvent.setup();
    const action = vi
      .fn<() => Promise<MutationResult<unknown>>>()
      .mockResolvedValueOnce({ ok: false, message: 'transient failure' })
      .mockResolvedValueOnce({ ok: true, data: null });
    const { onDone } = renderDialog({ action, successMessage: 'Session stopped' });

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));
    expect(await screen.findByText('transient failure')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: LABELS.confirmLabel }));

    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    // The stale error is gone and the success notice replaced it.
    expect(screen.queryByText('transient failure')).not.toBeInTheDocument();
    expect(await screen.findByText('Session stopped')).toBeInTheDocument();
    expect(action).toHaveBeenCalledTimes(2);
  });

  it('invokes onClose from the cancel button without running the action', async () => {
    const user = userEvent.setup();
    const { action, onClose } = renderDialog();

    await user.click(screen.getByRole('button', { name: LABELS.cancelLabel }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(action).not.toHaveBeenCalled();
  });
});
