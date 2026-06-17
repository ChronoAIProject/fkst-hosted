---
"fkst-hosted": patch
---

The NyxID service account is now optional: with auth enabled and a NyxID base URL set, the control plane builds an owner-only NyxID client from the user's token alone, enabling per-session NYXID_ACCESS_TOKEN provisioning and Ornn skill injection without NYXID_CLIENT_ID/SECRET.
