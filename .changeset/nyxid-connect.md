---
"fkst-hosted": minor
---

Add NyxID connect-at-install: an OAuth consent flow that captures a durable
per-owner broker `binding_id` into an in-memory store, so a webhook-triggered
session (user absent) can later mint NyxID credentials (milestone #9).
