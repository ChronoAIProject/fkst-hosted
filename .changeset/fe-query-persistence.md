---
"fkst-hosted": minor
---

feat(frontend): cross-session query persistence (PersistQueryClientProvider + IndexedDB)

Hydrate last-known successful API reads from IndexedDB on boot, then revalidate.
Only successful GET reads are dehydrated (no mutations/errors), maxAge 24h,
buster-versioned, gcTime >= maxAge, wiped on sign-out. ARCHITECTURE.md §4/§8.
