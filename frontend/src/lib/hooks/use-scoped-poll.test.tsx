import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import { useScopedPoll } from './use-scoped-poll';

/** A controllable fetcher: every call parks until the test resolves it. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

interface ProbeProps {
  cacheKey: string;
  enabled?: boolean;
  pollEnabled?: boolean;
  intervalMs?: number;
  fetcher: (signal: AbortSignal) => Promise<string>;
}

function Probe({ cacheKey, enabled = true, pollEnabled, intervalMs = 5000, fetcher }: ProbeProps) {
  const poll = useScopedPoll<string>({ key: cacheKey, intervalMs, enabled, pollEnabled, fetcher });
  return (
    <div>
      <span data-testid="data">{poll.data ?? 'none'}</span>
      <span data-testid="error">{poll.error ? String((poll.error as Error).message) : 'none'}</span>
      <span data-testid="loading">{String(poll.loading)}</span>
      <span data-testid="refreshing">{String(poll.refreshing)}</span>
      <button type="button" onClick={poll.refresh}>
        refresh
      </button>
    </div>
  );
}

const value = (id: string) => screen.getByTestId(id).textContent;

/** Drive the document's visibility, which the poll observes. */
function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, value: hidden });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('useScopedPoll', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setHidden(false);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('fetches immediately and publishes the payload for its key', async () => {
    const d = deferred<string>();
    render(<Probe cacheKey="k1" fetcher={() => d.promise} />);
    expect(value('loading')).toBe('true');
    await act(async () => {
      d.resolve('first');
    });
    expect(value('data')).toBe('first');
    expect(value('loading')).toBe('false');
  });

  it('drops the previous key’s data SYNCHRONOUSLY when the key changes', async () => {
    const first = deferred<string>();
    const { rerender } = render(<Probe cacheKey="k1" fetcher={() => first.promise} />);
    await act(async () => {
      first.resolve('alice rows');
    });
    expect(value('data')).toBe('alice rows');

    // A never-resolving fetcher for the new key: if the old data survived even
    // one frame, this assertion would catch it.
    rerender(<Probe cacheKey="k2" fetcher={() => new Promise<string>(() => {})} />);
    expect(value('data')).toBe('none');
  });

  it('aborts the in-flight request when the key changes', async () => {
    const seen: AbortSignal[] = [];
    const fetcher = (signal: AbortSignal) => {
      seen.push(signal);
      return new Promise<string>(() => {});
    };
    const { rerender } = render(<Probe cacheKey="k1" fetcher={fetcher} />);
    rerender(<Probe cacheKey="k2" fetcher={fetcher} />);
    expect(seen[0]?.aborted).toBe(true);
    expect(seen[1]?.aborted).toBe(false);
  });

  it('never lets a superseded response land over the current key', async () => {
    const stale = deferred<string>();
    const fresh = deferred<string>();
    let call = 0;
    const fetcher = () => (call++ === 0 ? stale.promise : fresh.promise);
    const { rerender } = render(<Probe cacheKey="k1" fetcher={fetcher} />);
    rerender(<Probe cacheKey="k2" fetcher={fetcher} />);
    await act(async () => {
      fresh.resolve('new scope');
      // The abandoned first request answers LAST — the classic race.
      stale.resolve('old scope');
    });
    expect(value('data')).toBe('new scope');
  });

  it('is single-flight: a poll tick during a request issues nothing new', async () => {
    const d = deferred<string>();
    const fetcher = vi.fn(() => d.promise);
    render(<Probe cacheKey="k1" intervalMs={1000} fetcher={fetcher} />);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(3000);
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it('queues AT MOST one refresh and issues it when the request settles', async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    let call = 0;
    const fetcher = vi.fn(() => (call++ === 0 ? first.promise : second.promise));
    render(<Probe cacheKey="k1" intervalMs={1000} fetcher={fetcher} />);

    // Three refresh requests arrive while the first is in flight.
    await act(async () => {
      screen.getByRole('button', { name: 'refresh' }).click();
      screen.getByRole('button', { name: 'refresh' }).click();
      screen.getByRole('button', { name: 'refresh' }).click();
    });
    expect(fetcher).toHaveBeenCalledTimes(1);

    await act(async () => {
      first.resolve('first');
    });
    // Exactly ONE follow-up, not three.
    expect(fetcher).toHaveBeenCalledTimes(2);
    await act(async () => {
      second.resolve('second');
    });
    expect(value('data')).toBe('second');
  });

  it('polls on the interval while visible, and stops while hidden', async () => {
    const fetcher = vi.fn(() => Promise.resolve('x'));
    render(<Probe cacheKey="k1" intervalMs={5000} fetcher={fetcher} />);
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(fetcher).toHaveBeenCalledTimes(2);

    await act(async () => {
      setHidden(true);
    });
    await act(async () => {
      vi.advanceTimersByTime(20000);
    });
    expect(fetcher).toHaveBeenCalledTimes(2);

    // Returning to the tab refreshes IMMEDIATELY: the data is stale by
    // definition after a pause.
    await act(async () => {
      setHidden(false);
    });
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it('honours a disabled timer while still keeping (and loading) the data', async () => {
    const fetcher = vi.fn(() => Promise.resolve('page one'));
    render(<Probe cacheKey="k1" intervalMs={1000} pollEnabled={false} fetcher={fetcher} />);
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    // The first load still happened; only the recurring timer is suspended.
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(value('data')).toBe('page one');
  });

  it('fetches nothing and retains nothing while disabled', async () => {
    const fetcher = vi.fn(() => Promise.resolve('x'));
    const { rerender } = render(<Probe cacheKey="k1" fetcher={fetcher} />);
    await act(async () => {});
    expect(value('data')).toBe('x');
    rerender(<Probe cacheKey="k1" enabled={false} fetcher={fetcher} />);
    expect(value('data')).toBe('none');
  });

  it('keeps the last-good frame alongside a failure for the SAME key', async () => {
    const good = deferred<string>();
    const bad = deferred<string>();
    let call = 0;
    const fetcher = () => (call++ === 0 ? good.promise : bad.promise);
    render(<Probe cacheKey="k1" intervalMs={1000} fetcher={fetcher} />);
    await act(async () => {
      good.resolve('snapshot');
    });
    await act(async () => {
      bad.reject(new Error('boom'));
      vi.advanceTimersByTime(1000);
    });
    expect(value('data')).toBe('snapshot');
    expect(value('error')).toBe('boom');
  });

  it('does not report a cancellation as a failure', async () => {
    const abortError = Object.assign(new Error('aborted'), { name: 'AbortError' });
    const d = deferred<string>();
    render(<Probe cacheKey="k1" fetcher={() => d.promise} />);
    await act(async () => {
      d.reject(abortError);
    });
    expect(value('error')).toBe('none');
  });
});
