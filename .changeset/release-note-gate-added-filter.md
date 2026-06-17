---
---

Fix the `require-release-note` gate so it validates only the note **added** by
the PR. It previously grepped every changed path (added, modified, or removed)
and validated whichever sorted first, so the previous release's removed note
(its earlier timestamp sorts first) was picked over the note the release
actually adds — failing every `develop → main` PR that cleans up a prior note.
CI-only; no product behaviour change. Closes #249.
