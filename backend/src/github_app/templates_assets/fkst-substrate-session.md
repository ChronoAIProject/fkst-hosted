---
name: fkst substrate session
about: Start an fkst agent session against this repository.
title: "[session] "
labels: [fkst-substrate-trigger]
---

<!--
Opening this issue starts an fkst session. fkst parses the sections below by
their EXACT `### ` headings. Do NOT rename the headings. Do NOT put secrets,
tokens, or credentials anywhere in this issue — they are supplied only through
your environment and are never read from issue text.

⚠️ ONE-SHOT CONFIG: the moment the session registers (the 🟢 comment appears),
EVERYTHING in this issue body is FROZEN. Edits after that are rejected with an
`fkst-config-rejected` label and change nothing. To change any setting, close
this issue and open a new trigger.
-->

### Session Name

<!-- One line. Lower-case DNS-label: ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$, 1-40 chars. -->
my-first-session

### Packages

<!--
One package reference per line, each as `owner/repo@ref:path/to/package`, pointing
at a PUBLIC repo that has an `fkst.toml` at that path. At least one is required.
-->
ChronoAIProject/fkst-packages@main:packages/example

### Work Label

<!--
One line. The GitHub label whose OPEN issues are this session's work queue. Max 50
characters, no commas. Create work items as separate issues carrying this label
(see the "fkst work item" template).
-->
fkst-work

### Environment

<!--
Optional. One line naming an environment you created via
`PUT /api/v1/users/me/environments/<name>`. Delete this whole section to run with
no environment.
-->

### Auto-merge

<!--
Optional. Set to `true` to have fkst automatically merge THIS session's pull
requests into the default branch as soon as GitHub reports them mergeable. This
BYPASSES review and status checks — use it only on trusted repos. Accepted truthy
values: true / yes / on / enabled / 1 (case-insensitive). Anything else, or
deleting this section, leaves auto-merge OFF (the default).
-->
false

### FKST Contributors

<!--
Optional TRUSTED-USERS list, doing two things at once. List extra GitHub logins
— one per line, or comma/space separated; a leading `@` is fine — that you trust
on this session IN ADDITION to you (the issue author), who is always included:
1. The session ONLY acts on issues/comments written by its contributors (you,
   these users, and the bot). Anyone else's input is ignored by the substrate.
2. These users may also DOWNLOAD the session's redacted logs (log streaming is
   always on; global admins can always download).
Numeric user ids are also accepted, but only for log download — the substrate
matches contributors by LOGIN. Delete this whole section to trust only yourself.
(The old heading `### Log Access Allowlist` is still accepted as an alias.)
-->

### Session Collaborators

<!--
Optional WORK-ITEM AUTHORITY list. List extra GitHub logins — one per line, or
comma/space separated; a leading `@` is fine — granted authority over THIS
session's work issues IN ADDITION to you (the issue author), who always has it.
These users may raise, label, and comment on the session's work issues (its work
queue). Delete this whole section to grant authority to only yourself. (Frozen at
registration like every other setting in this issue.)
-->

### Output Language

<!--
Optional. ONE locale tag (e.g. `en`, `zh`, `zh-CN`) — the language the session's
packages write user-visible comments in. The value must EXACTLY match a
`locales/<value>.lua` file shipped by the session's package (case- and
separator-sensitive); a mismatch silently falls back to English. Delete this
whole section for English (the default).
-->

### Engine Config

<!--
Optional. Advanced engine tunables, ONE `KEY=value` per line, drawn ONLY from
this allowlist (anything else is rejected with an explanatory comment):
  FKST_CODEX_PERMIT_SLOTS=<1..32>          concurrent codex subprocesses
  FKST_QUEUE_CAPACITY=<1..1024>            per-queue capacity
  FKST_MAX_IN_FLIGHT_PER_DEPT=<1..1024>    per-department concurrency
  FKST_DURABLE_ADMISSION_BURST_PER_DEPT=<1..1024>
  FKST_RETRY_DEFAULT_MAX_ATTEMPTS=<1..100>
  FKST_RETRY_DEFAULT_BASE / FKST_RETRY_DEFAULT_CAP /
  FKST_DEPARTMENT_DEFAULT_STALL_WINDOW /
  FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=<n>s|m|h   (1 second ..= 7 days; the
    effective retry CAP must stay >= the effective BASE — defaults 60s / 30m)
  FKST_RATE_POOL_<NAME>=<burst>,<refill_per_minute> throttle a program by
    basename (e.g. FKST_RATE_POOL_GH=10,10); platform defaults can only be
    TIGHTENED, never widened.
Delete this whole section for the engine defaults.
-->
