# workflow-security step prompts

The authoritative per-step codex instructions live in the built-in template's
`generator` fields (`../records.lua` and `../blueprints/security-review.json`).
These mirrors are for humans reviewing the pipeline.

- `profile-stack.md` — enumerate declared dependencies from manifests/lockfiles.
- `match-dependencies.md` — match dependencies against GitHub Security Advisories via `gh api` (zero new egress).
- `audit-code-tests.md` — audit code, tests and security best practices.
- `file-findings.md` — consolidate findings into the strict JSON array the engine files as issues.
