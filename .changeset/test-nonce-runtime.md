---
"fkst-hosted": patch
---

Use a runtime-random per-session nonce in the git-credential-helper JIT tests instead of hard-coded literals. This clears two false-positive CodeQL "hard-coded cryptographic value" alerts (surfaced when the #145 crate rename moved the test file) without weakening the test — it now exercises an arbitrary nonce on every run.
