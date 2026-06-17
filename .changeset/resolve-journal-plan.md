---
"fkst-hosted": patch
---

Populate the dispatch `JournalPlan` controller-side (#185, #151 increment 6b). `resolve_dispatch` projects the controller's process `JournalConfig` (`inner.journal`) into the wire `JournalPlan`, shipping `Some(plan)` only when journaling would write a durable record — `github_enabled` with both a repo and a token, byte-for-byte the gate `fkst_journal::Journaler::start` applies — and `None` otherwise. So the worker journals for exactly the configs the in-process driver produces a durable record for. Still dormant (no dispatch is emitted yet; the in-process driver is untouched). Also re-exports `JournalPlan` from `fkst_shared::protocol` (an increment-6a gap that only surfaced at the first cross-module consumer).
