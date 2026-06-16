---
"fkst-hosted": patch
---

Add the dormant `JournalPlan` dispatch wire type (#183, #151 increment 6a). `ResolvedDispatch` gains an `Option<JournalPlan>` field carrying the process-level `fkst_journal::JournalConfig` fields (flush cadence, repo/branch/api-base, identity pointers, line cap, and the journal-repo `github_token` as a `SecretString`) so the controller can ship a fully-resolved journaling plan to the worker — keeping the worker free of any standing journal secret. `None` means journaling is disabled for the run. Nothing constructs a non-`None` plan yet (the controller populates it in 6b; the worker consumes it in 6c), so this is additive and behavior-preserving. The journal token redacts in `Debug` and is exposed only on the wire.
