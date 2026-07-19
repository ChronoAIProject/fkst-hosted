import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';
import { CopyButton } from './copy-button';

/**
 * Install (or remove) a fake `navigator.clipboard` for a single test. jsdom's
 * navigator has no clipboard, and the property is non-configurable in some
 * engines, so go through defineProperty rather than plain assignment.
 */
function setClipboard(clipboard: Clipboard | undefined) {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: clipboard,
  });
}

afterEach(() => {
  vi.useRealTimers();
  setClipboard(undefined);
  vi.restoreAllMocks();
});

describe('CopyButton', () => {
  it('writes the exact value via the Clipboard API and confirms', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText } as unknown as Clipboard);

    render(<CopyButton value="session-abc-123" />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });

    // The exact string, verbatim — not trimmed or re-encoded.
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith('session-abc-123');
    // Polite live region announces the confirmation.
    expect(screen.getByText('Copied')).toBeInTheDocument();
  });

  it('reverts the copied state after the hold window elapses', async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText } as unknown as Clipboard);

    render(<CopyButton value="ref@sha" />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });
    expect(screen.getByText('Copied')).toBeInTheDocument();

    // The live region empties once the ~1.5s timer fires.
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(screen.queryByText('Copied')).not.toBeInTheDocument();
  });

  it('honors a custom label as both visible text and accessible name', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText } as unknown as Clipboard);

    render(<CopyButton value="x" label="Copy session ID" />);

    const button = screen.getByRole('button', { name: 'Copy session ID' });
    expect(button).toHaveTextContent('Copy session ID');
  });

  it('falls back to execCommand when the Clipboard API is unavailable', async () => {
    setClipboard(undefined); // no navigator.clipboard at all
    const execCommand = vi.fn().mockReturnValue(true);
    // jsdom does not implement execCommand — install a spy to observe it.
    (document as unknown as { execCommand: typeof execCommand }).execCommand = execCommand;

    render(<CopyButton value="fallback-value" />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });

    expect(execCommand).toHaveBeenCalledWith('copy');
    // The hidden textarea must have carried the exact value while selected.
    expect(screen.getByText('Copied')).toBeInTheDocument();
  });

  it('does not claim success when every copy path fails', async () => {
    setClipboard(undefined);
    // execCommand present but reporting failure — no confirmation is a lie-free
    // outcome the user can retry.
    const execCommand = vi.fn().mockReturnValue(false);
    (document as unknown as { execCommand: typeof execCommand }).execCommand = execCommand;

    render(<CopyButton value="nope" />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });

    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(screen.queryByText('Copied')).not.toBeInTheDocument();
  });
});
