import { get, set, del } from 'idb-keyval';
import { createAsyncStoragePersister } from '@tanstack/query-async-storage-persister';

/**
 * Cross-session persistence for TanStack Query (ARCHITECTURE.md §4/§8).
 *
 * On boot the console hydrates last-known **successful** API reads from IndexedDB
 * and paints instantly, then revalidates — a labelled, re-derivable snapshot,
 * NEVER presented as live and NEVER a second source of truth. Only successful GET
 * reads are dehydrated; mutations, errors, and `unknown` placeholders are not.
 */

/** IndexedDB key for the dehydrated query client. */
const IDB_KEY = 'fkst:query-cache';

/** Evict persisted entries older than this on restore (stale snapshots are dropped). */
export const PERSIST_MAX_AGE = 1000 * 60 * 60 * 24; // 24h

/**
 * Cache version. Bump to invalidate ALL persisted caches at once (e.g. when a
 * response shape changes). `buster` mismatch → the persisted client is discarded.
 */
export const PERSIST_BUSTER = 'v1';

/** idb-keyval-backed AsyncStorage adapter (IndexedDB, not localStorage — size + leak safety). */
const idbStorage = {
  getItem: async (key: string): Promise<string | null> => (await get<string>(key)) ?? null,
  setItem: (key: string, value: string): Promise<void> => set(key, value),
  removeItem: (key: string): Promise<void> => del(key),
};

export const queryPersister = createAsyncStoragePersister({
  storage: idbStorage,
  key: IDB_KEY,
  throttleTime: 1000,
});

/**
 * Wipe the persisted cache. Called on sign-out so one identity's cached
 * goal/GitHub reads never bleed into the next (identity-scoped clear, §8).
 */
export async function clearPersistedCache(): Promise<void> {
  await del(IDB_KEY);
}
