import { useCallback, useEffect, useRef, useState } from 'react';
import { useVisibilityPoll } from './use-visibility-poll';

/** Whether a rejection is a cancellation this hook itself requested. */
function isAbort(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'name' in error &&
    (error as { name?: unknown }).name === 'AbortError'
  );
}

/**
 * A visibility-aware, single-flight poll whose result set is BOUND to a cache
 * key. The operations views plug into it; nothing about it is operations-
 * specific, which is the point — the rules below are general enough that any
 * per-viewer, per-scope feed can adopt them without re-deriving them.
 *
 * The five rules, each earned by a failure this surface must not have:
 *
 * - **The key owns the data.** `data` is returned only when it was produced for
 *   the CURRENT key. The check is a render-time comparison, not an effect, so a
 *   sign-out, an account switch, a scope change, or a filter edit drops the old
 *   rows *synchronously* — there is no frame in which the previous viewer's data
 *   is on screen under the new key.
 * - **Single flight.** One request per key at a time. A view whose response is
 *   slower than its poll interval must not stack requests; the queued-refresh
 *   slot below is what keeps a user-initiated refresh from being lost to that.
 * - **At most one queued refresh.** A refresh arriving mid-flight is remembered
 *   once and issued when the current one settles. Two arriving are still one:
 *   the second would fetch the same thing.
 * - **Superseded requests are aborted, not ignored.** Ignoring a response still
 *   pays for it; aborting also stops a slow page from holding a connection while
 *   the user moves on.
 * - **Errors never silently keep stale data.** A failure keeps the last-good
 *   frame ONLY under an unchanged key, and always alongside the error, so the
 *   caller can render "this is what we last saw, and here is why it stopped".
 */
export interface ScopedPollResult<T> {
  /** The last successful payload for the CURRENT key, or `null`. */
  data: T | null;
  /** The failure of the most recent attempt for the current key, or `null`. */
  error: unknown;
  /** True while the first request for this key is in flight with nothing to
   *  show — the only state that should render a skeleton. */
  loading: boolean;
  /** True while any request for this key is in flight. */
  refreshing: boolean;
  /** `Date.now()` when `data` landed. `null` when there is no data. */
  updatedAt: number | null;
  /** Request a refresh now. Coalesced into the queue slot while one is in
   *  flight. */
  refresh: () => void;
}

interface Snapshot<T> {
  key: string;
  data: T | null;
  error: unknown;
  updatedAt: number | null;
}

export interface ScopedPollOptions<T> {
  /** The result set's identity. Changing it clears data synchronously. */
  key: string;
  /** Poll cadence while enabled and the document is visible. */
  intervalMs: number;
  /** When false, nothing is fetched and nothing is retained. */
  enabled: boolean;
  /**
   * Whether the recurring timer runs. Defaults to `enabled`.
   *
   * It is separate because "stop refreshing" and "forget what you have" are
   * different instructions: a caller reading an older page wants the first page
   * frozen, not discarded, and clearing it would destroy the very investigation
   * the pause exists to protect.
   */
  pollEnabled?: boolean;
  /** Perform one request. Must reject on failure and honour the signal. */
  fetcher: (signal: AbortSignal) => Promise<T>;
}

export function useScopedPoll<T>({
  key,
  intervalMs,
  enabled,
  pollEnabled,
  fetcher,
}: ScopedPollOptions<T>): ScopedPollResult<T> {
  const [snapshot, setSnapshot] = useState<Snapshot<T>>({
    key,
    data: null,
    error: null,
    updatedAt: null,
  });
  const [inFlightKey, setInFlightKey] = useState<string | null>(null);

  // Live mirrors so the fetch closure never has to be rebuilt (and so the poll
  // timer is never restarted) just because a value it reads changed.
  const keyRef = useRef(key);
  keyRef.current = key;
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const requestIdRef = useRef(0);
  const activeRef = useRef<{ key: string; abort: AbortController } | null>(null);
  const queuedRef = useRef(false);

  const start = useCallback(() => {
    if (!enabledRef.current) return;
    const forKey = keyRef.current;
    if (activeRef.current) {
      // Single flight. A refresh that arrives mid-request is remembered exactly
      // once; the key check on settle decides whether it is still wanted.
      queuedRef.current = true;
      return;
    }
    const requestId = ++requestIdRef.current;
    const abort = new AbortController();
    activeRef.current = { key: forKey, abort };
    setInFlightKey(forKey);

    /** A response may land only when it is still the newest request AND the key
     *  has not moved on. Both are required: a superseded request for the same
     *  key must not overwrite a newer one either. */
    const isCurrent = () => requestIdRef.current === requestId && keyRef.current === forKey;

    fetcherRef
      .current(abort.signal)
      .then((data) => {
        if (!isCurrent()) return;
        setSnapshot({ key: forKey, data, error: null, updatedAt: Date.now() });
      })
      .catch((error: unknown) => {
        // An abort is this hook's own doing, never a failure to report.
        if (isAbort(error)) return;
        if (!isCurrent()) return;
        // Keep the last-good frame for THIS key alongside the error; a key
        // change would already have dropped it.
        setSnapshot((prev) =>
          prev.key === forKey
            ? { ...prev, error }
            : { key: forKey, data: null, error, updatedAt: null }
        );
      })
      .finally(() => {
        if (activeRef.current?.abort !== abort) return;
        activeRef.current = null;
        setInFlightKey((current) => (current === forKey ? null : current));
        if (queuedRef.current) {
          queuedRef.current = false;
          // Only worth issuing while the feed is still wanted.
          if (enabledRef.current) start();
        }
      });
  }, []);

  // A key change (identity, scope, filters) or a disable ABORTS whatever is in
  // flight: its answer describes a question nobody is asking any more.
  useEffect(() => {
    queuedRef.current = false;
    activeRef.current?.abort.abort();
    activeRef.current = null;
    setInFlightKey(null);
    if (!enabled) return;
    start();
    return () => {
      queuedRef.current = false;
      activeRef.current?.abort.abort();
      activeRef.current = null;
    };
  }, [key, enabled, start]);

  useVisibilityPoll(start, intervalMs, enabled && (pollEnabled ?? true));

  // The synchronous clear: data produced under a different key is not this
  // key's data, whatever the effects have or have not run yet.
  const current = snapshot.key === key && enabled;
  const data = current ? snapshot.data : null;
  const error = current ? snapshot.error : null;
  const refreshing = inFlightKey === key;

  return {
    data,
    error,
    loading: refreshing && data === null && error === null,
    refreshing,
    updatedAt: current ? snapshot.updatedAt : null,
    refresh: start,
  };
}
