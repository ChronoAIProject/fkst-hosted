import type { ObserveSnapshot } from '@/lib/api/types';

/** The on-demand observe fetch state, lifted to the drawer so the Status tab
 *  (which triggers it) and the Packages tab (which surfaces per-queue activity)
 *  share one fetch instead of each calling the slow pod-exec endpoint. */
export type ObserveState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error' }
  | { status: 'loaded'; snapshot: ObserveSnapshot };
