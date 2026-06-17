---
"fkst-hosted": patch
---

Remove the now-dead RFC 8693 token-exchange machinery and the NyxID service-account credential path: the control plane now builds an owner-only NyxID client (forwarded user token only) and no longer reads NYXID_CLIENT_ID/SECRET.
