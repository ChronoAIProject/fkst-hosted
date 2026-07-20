import type { ObserveSnapshot } from '@/lib/api/types';

/** The on-demand observe fetch state, lifted to the drawer so the Status tab
 *  (which triggers it) and the Packages tab (which surfaces per-queue activity)
 *  share one fetch instead of each calling the slow pod-exec endpoint. */
export type ObserveState =
  | { status: 'idle' }
  | { status: 'loading' }
  // `httpStatus` carries the failed request's HTTP status (from `ObserveError`)
  // so the Status tab can explain itself — e.g. 409 == no durable delivery store
  // to observe — instead of a bare red line. Undefined when the failure carried
  // no status (network error / non-`ObserveError` throw).
  | { status: 'error'; httpStatus?: number }
  | { status: 'loaded'; snapshot: ObserveSnapshot };
