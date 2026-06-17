---
"fkst-hosted": patch
---

Harden the goal-session GitHub token: hold it in `secrecy::SecretString` end-to-end with a redacting `Debug` on `GoalEnv` (exposed only at the child-env set-site), and re-mint reactively on a detected GitHub auth failure (bypassing the 55-min interval gate, respecting the 60s cooldown) instead of waiting for the time-based loop to catch the expiry.
