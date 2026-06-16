---
"fkst-hosted": patch
---

Add the `fkst-goal` GitHub Issue Form (`.github/ISSUE_TEMPLATE/fkst-goal.yml`) and document its canonical parse contract (#177). A user fills the form's three sections — **Goal**, **Package Name List**, **Ornn Skill List** — and `docs/api-reference.md` now defines exactly how the backend extracts them: the parser splits the body on the `### ` headings, the Goal becomes the (secret) engine prompt, package names follow `validate_goal_fields`/`resolve_package_roots` (`^[A-Za-z0-9_-]+$`, 1–16, no dupes, `host` reserved), and Ornn pins follow `kind:name@major.minor` validated by `validate_pins` — with malformed input mapping to `422`. Template + documentation only; the submit endpoint that consumes the contract and the `fkst-goal` label are sibling milestone-#5 issues. The form declares no labels and the security note forbids putting secrets in the issue.
