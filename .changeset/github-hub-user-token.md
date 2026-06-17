---
"fkst-hosted": patch
---

Route the github_hub credential seam through the forwarded user token instead of an RFC 8693 delegated token, so a rejected/expired token now surfaces as 401 rather than a 503.
