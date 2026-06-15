---
"fkst-hosted": patch
---

Turn opaque GitHub repo-creation auth failures into actionable 422s: a connection missing the `repo` OAuth scope, an org SAML-SSO authorization gate (surfacing the authorization URL), and org-policy/visibility denials are now distinguished from genuine credential failures and explained to the user. Documents that the `repo` scope grant is NyxID-side configuration (provider `default_scopes` or a `github-pat`).
