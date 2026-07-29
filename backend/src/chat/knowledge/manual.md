# fkst-hosted — concierge knowledge base

This is the concierge's grounding source for how the platform behaves. It is curated
from the repository's canonical sources: the user manual skill, the authoritative
status-label table, and the trigger-issue template the GitHub App actually installs.
It is deployment-agnostic — relative API paths only, no hostnames, no operator or
cluster content.

## What fkst-hosted is, and the three control surfaces

fkst-hosted runs long-lived autonomous coding sessions anchored by GitHub issues:
open a trigger issue to start a session, queue tasks as more issues, and each task
comes back as its own pull request.

There are three control surfaces, and they drive the same underlying state:

- **GitHub issues** — the durable source of truth for session declarations, work
  queues, status, and stopping. Never put secrets, commands, or environment values in
  an issue.
- **The web dashboard** — a visual companion for the same operations: sign in with
  GitHub to see every account, repository and session; start sessions, queue work,
  stop sessions, manage environments and App installations, and read logs and outcomes
  in the browser.
- **The REST API** — the machine-readable dashboard surface under `/api/v1/*`. In
  particular `POST /api/v1/repos/{owner}/{name}/sessions` is the only non-UI way to
  submit a disposable environment privately while the API opens the corresponding
  trigger issue as the authenticated user.

The trigger issue stays the durable declaration even when the dashboard or the API
creates it.

### Vocabulary

| Concept | Is a… | You control it by… |
|---|---|---|
| **Session** | one long-lived coding agent bound to a repository | opening/closing a **trigger issue** (or dashboard New session / Stop) |
| **Trigger issue** | the session's declaration: name, packages/manifests, labels, options | an issue labeled `fkst-substrate-trigger` |
| **Work item** | one task for the session — it becomes a pull request | an issue carrying one of the session's **work labels** (or dashboard "Add work item") |
| **Run** | one incarnation of a session's runtime (sessions sleep and revive) | nothing — but each run keeps its own downloadable log bundle |

One trigger issue means one session. Open work-label issues are the queue that
session works, each as its own pull request.

### Timing — nothing is instant

Actions take effect on a short platform sweep: expect **seconds (~30 s)**, not
immediate feedback. A brand-new repository can take up to **~10 minutes** to be
noticed if webhook delivery is not active. When a user reports "nothing happened",
the first question is how long ago they acted — re-checking the issue's comments and
labels after a sweep is usually the answer.

## Install the GitHub App and the auto-seeded starter session

Sessions only run on repositories where the fkst-hosted GitHub App is installed.
Install it from the dashboard's **Install the GitHub App / Connect / Install** buttons
(they deep-link to GitHub's install page) or directly on GitHub.

- A user who is not an admin of the target repository may have their install routed to
  the owner as an approval request.
- **Auto-seeded starter session** (enabled by default): on install, each repository
  with no open trigger issue gets one auto-created trigger issue —
  `[session] default-workflows (auto-seeded)` — referencing the platform's default
  workflow manifest, with Auto-merge on, listing the installing account as an FKST
  Contributor. It behaves like any other trigger: its config freezes once registered,
  and closing it retires the session. Close or delete it if the default workflows are
  not wanted.
- The App also installs two issue templates, kept up to date automatically: **"fkst
  substrate session"** (a pre-labeled trigger scaffold) and **"fkst work item"** (a task
  scaffold). Blank issues are disabled in favor of them.

### Re-attributing an App-seeded trigger

An auto-seeded trigger is authored by the App, not a human. Its **effective creator is
its sole assignee**. To re-attribute one: remove every existing assignee and assign
**exactly the intended creator**. Zero or multiple assignees are not attributable and
the trigger is rejected before its body is read. That creator must also be a
deployment global admin or hold repository **admin or maintain** permission.

## Trigger-issue grammar — the full reference

A trigger is a GitHub issue labeled `fkst-substrate-trigger` (the "fkst substrate
session" template applies the label). The body is a set of `### ` sections matched by
**exact heading**:

- Text before the first heading is ignored.
- A **duplicate heading makes the issue invalid**.
- `#### ` and deeper are ordinary text.
- HTML comments (`<!-- … -->`) are ignored, so the template can be filled in place.

| Section | Required? | Rule |
|---|---|---|
| `### Session Name` | **yes** | exactly one line: lowercase letters, digits, and inner dashes (`my-session`), 1–40 chars |
| `### Packages` | one of these two | zero or more lines, each a package reference `owner/repo@ref:path` |
| `### Manifest` | one of these two | zero or more lines, each a manifest reference `owner/repo@ref:path` pointing at an fkst-manifest JSON file |
| `### Work Label` | optional | **exactly one** label, ≤ 50 characters, no comma. Omit or leave blank to auto-discover labels from the session's packages |
| `### Environment` | optional | the name of **one** reusable environment profile the trigger author has saved. Selects the profile only — never put commands, variable values, or secrets in an issue |
| `### Source Branch` | optional | upstream branch used to seed a missing target and receive completed target work; omit for the repository's default branch |
| `### Target Branch` | optional | integration branch the session works against; omit for `fkst-hosted-default` |
| `### Auto-merge` | optional | `true` / `yes` / `on` / `enabled` / `1` (case-insensitive) turns it on; anything else is off. When on, the App bot's pull requests auto-merge to the default branch once mergeable and the linked work issue is closed — this bypasses the repository's review and checks flow |
| `### FKST Contributors` | optional | the session's trusted users (the author is always included): the session acts only on issues and comments from these people, and they may download its logs. GitHub logins and/or numeric ids separated by spaces, commas or newlines; a leading `@` is fine; numeric ids count for log access only |
| `### Log Access Allowlist` | optional | a permanent alias of `### FKST Contributors`. Both may appear and the lists merge |
| `### Session Collaborators` | optional | people granted **work-item authority** — they may raise, label and comment on this session's work issues. Distinct from log access, and they deliberately cannot stop the session. Same list format |
| `### Output Language` | optional | one locale tag (`en`, `zh`, `zh-CN`, …). It must exactly match a locale the session's package ships, or output silently falls back to English |
| `### Engine Config` | optional | advanced tunables, one `KEY=value` per line from a strict allowlist. Any other key makes the trigger invalid |

**At least one package source is required.** A trigger with neither `### Packages` nor
`### Manifest` is invalid: "the trigger must list at least one package source".

Do not set the output language in `### Engine Config` — a dedicated error says so.

## Package and manifest references — the `owner/repo@ref:path` grammar

Both `### Packages` and `### Manifest` use the same reference form,
`owner/repo@ref:path/to/package`. A reference splits at the **first `@`** (into
`owner/repo` and the rest) and then the **first `:`** (into `ref` and `path`):

- `owner`, `repo` — letters, digits, `.`, `_`, `-`, with exactly one `/` between them.
- `ref` — a branch, tag, or commit SHA; no `..` segments.
- `path` — repository-relative, never absolute, no `..`. For `### Packages` it points at
  a **package directory**; for `### Manifest` it points at the **manifest JSON file
  itself**.

Every referenced repository must be **public** and must contain the expected content at
that ref and path. An unreachable reference blocks the session and the bot comments
listing exactly which refs failed and why.

## Manifests — bundles that expand into packages

A manifest is a JSON file — `{"schemaVersion": 1, "name": …, "packages": [ …refs… ]}`
with 1–64 package references — that the platform expands into a package list.

- A manifest is sufficient on its own: a session may reference only a manifest.
- Both sections may be combined. The effective package set is the explicit
  `### Packages` lines first, then each manifest's packages, de-duplicated, with an
  explicit entry keeping its position.
- A manifest that cannot be fetched or does not validate makes the trigger invalid. It
  never partially applies.

## Work labels — one explicit, many discovered, exclusive per creator

- The explicit `### Work Label` is at most **one** label per trigger.
- Packages can declare their own work labels. A session's **effective label set** is
  the explicit label plus every label its packages declare. The registration comment
  lists the full set, and the session picks up issues carrying **any** of them.
- If `### Work Label` is omitted and the packages declare none, the trigger is flagged
  invalid: *"no work label: add a `### Work Label` section or use packages that declare
  work labels"*. Adding a label (or using packages that declare labels) clears the flag.
- **Collisions**: each label belongs to the **oldest open trigger** (lowest issue
  number) owned by the same creator that uses it. A newer trigger colliding on any of
  its labels is flagged invalid — *"work label 'x' collides with active session #N"* —
  until the older session closes or the label changes. Different creators may
  deliberately use overlapping labels, because assignees route their work
  independently. The dashboard's New-session dialog pre-checks explicit-label
  collisions and warns before submission.

## `### Engine Config` — the allowlisted tunables

One `KEY=value` per line, no duplicates. Any key outside this list makes the trigger
invalid.

| Key | Accepted values |
|---|---|
| `FKST_LLM_MODEL` | a plain model id served by the deployment's LLM endpoint (letters, digits, `. _ / : -`; ≤ 128 chars) — runs THIS session on that model instead of the deployment default |
| `FKST_LLM_REASONING_EFFORT` | `minimal` \| `low` \| `medium` \| `high` \| `max` (case-insensitive, stored lowercase; deployment default `max`) |
| `FKST_CODEX_PERMIT_SLOTS` | integer 1–32 |
| `FKST_QUEUE_CAPACITY`, `FKST_MAX_IN_FLIGHT_PER_DEPT`, `FKST_DURABLE_ADMISSION_BURST_PER_DEPT` | integer 1–1024 |
| `FKST_RETRY_DEFAULT_MAX_ATTEMPTS` | integer 1–100 |
| `FKST_RETRY_DEFAULT_BASE`, `FKST_RETRY_DEFAULT_CAP`, `FKST_DEPARTMENT_DEFAULT_STALL_WINDOW`, `FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET` | a duration like `30s` / `5m` / `2h` (1 second – 7 days); the effective retry cap must stay ≥ the base (defaults 60s / 30m) |
| `FKST_RATE_POOL_<NAME>` | `<burst>,<refill_per_minute>`, both ≥ 1; `NAME` is `A–Z0–9_` (`ROOT` reserved). Platform pool defaults can only be **tightened**, never widened |

## A complete trigger-issue example

A minimal, valid trigger body:

```
### Session Name
sitebuilder

### Manifest
ChronoAIProject/fkst-hosted@packages:manifests/default-workflows.json

### Work Label
site-build

### Auto-merge
true
```

A fuller one exercising the optional sections:

```
### Session Name
docs-refresh

### Packages
ChronoAIProject/fkst-hosted@packages:packages/workflow-writer

### Work Label
docs-work

### Environment
my-node-env

### Source Branch
main

### Target Branch
docs-integration

### Auto-merge
false

### FKST Contributors
@alice @bob

### Session Collaborators
@carol

### Output Language
en

### Engine Config
FKST_LLM_REASONING_EFFORT=high
```

From the command line:

```
gh issue create --repo <owner>/<repo> \
  --title "[session] sitebuilder" \
  --body-file body.md \
  --label fkst-substrate-trigger
```

Or in the dashboard: repository workspace → **New session**, which offers the same
fields as a form (session name, package rows, manifests, work label with a live
collision warning, an environment mode of None / Saved profile / Disposable, source and
target branches, auto-merge, contributors, collaborators, output language).

## Registration and config immutability

When a trigger is accepted the bot posts the registration comment —
**🟢 **fkst session `name` registered.**** — listing the session's full effective work
label set, its packages, environment, auto-merge state and (when configured) the
**📥 Logs** URL. It ends with **"🔒 Config frozen."** and the trigger gains the
`fkst-substrate-active` label.

From that moment the session's entire configuration is **immutable**:

- Editing the trigger body afterwards changes nothing. The running session keeps its
  original config, a one-time comment explains the rejection, and the durable
  `fkst-config-rejected` label is added.
- **To change anything, close the trigger and open a new one.**
- Edits made *before* the 🟢 comment appears are fine.
- This is also why access lists cannot be widened retroactively.

## Queue and route work

Open one issue **per task**, carrying one of the session's work labels (the "fkst work
item" template gives a scaffold; the label is what routes it). Give each task a clear
title, the exact files to change, real acceptance criteria, and enough context to be
worked in isolation — the agent sees that one issue plus the repository, not the
sibling backlog.

The session acknowledges the claim with **👀 **Picked up by fkst session `name`.**** plus
the `fkst-picked-up` label, works the task, and opens a pull request for it
(auto-merged and auto-closed when the session has Auto-merge on).

From the dashboard: repository workspace → session card → **Add work item** (title plus
optional Markdown details). That action is available when the session has an explicit
`### Work Label`; auto-discovered-label sessions queue work by labeling issues on
GitHub.

### The routing rule — exactly one assignee

A labeled issue is only worked when it has **exactly one assignee, and that assignee is
the creator of an active session watching one of the issue's labels**. Zero assignees,
several assignees, or the wrong assignee all leave the issue unrouted (see
`fkst-unrouted`). Correcting the assignee is enough — the reconciler routes the issue
automatically on a later sweep.

### Who may author work

Work-issue authority is **always enforced; there is no opt-out flag.** Only these may
author work for a session:

- the session's **creator**,
- logins frozen under the session's `### Session Collaborators`,
- the deployment's **global admins**.

The configured fkst GitHub App is separately accepted as the system author of
workflow-generated child issues. Repository admin or maintain permission alone is **not**
work authority.

### Open work keeps the session alive

The reconciler's pending gate counts **open work-label issues**, not un-PR'd ones. A
created-but-unmerged pull request does not let a session idle down. Merge or close
finished work to let it wind down.

## Status labels and comments — the full state machine

A user applies only `fkst-substrate-trigger` and their own work labels. Every other
`fkst-*` label is managed by the platform.

Two kinds of label matter: a **clearable latch** self-heals once the underlying problem
is fixed (no new issue needed), while a **permanent** one is a durable record.

| Label | Where | Meaning and recovery | Latch |
|---|---|---|---|
| `fkst-substrate-trigger` | trigger issue | the label a user applies to declare a session | user-applied |
| `fkst-substrate-active` | trigger issue | the session registered; config is now frozen | permanent acknowledgement |
| `fkst-picked-up` | work issue | the session claimed this task | permanent acknowledgement (a stale latch is removed on retirement) |
| `fkst-trigger-unauthorized` | trigger issue | the trigger's effective creator is unattributable, or lacks global-admin / repository admin-or-maintain authority. A human-authored trigger uses its author; a bot-authored trigger needs **exactly one** assignee. Fix the assignee or grant the role; the body is not even parsed until then | clearable |
| `fkst-substrate-invalid` | trigger issue | the accepted trigger cannot parse or resolve, has no effective work label, or overlaps another active trigger owned by the same creator. The comment names the exact problem. Fix the body | clearable |
| `fkst-unrouted` | work issue | the labeled issue has zero or multiple assignees, or its sole assignee is not the creator of a matching active session. Correct the assignee | clearable |
| `fkst-unauthorized` | work issue | the routed issue's author is neither the configured fkst App nor the session's creator, a Session Collaborator, or a global admin. It stays unworked until the author becomes authorized | clearable |
| `fkst-degraded` | trigger issue | the session's own framework or pod reports a problem. Health is re-checked about every 2.5 minutes and the label clears when it recovers | clearable |
| `fkst-config-rejected` | trigger issue | the trigger body of a registered session was edited; the edit was ignored. Close it and open a new trigger to change configuration | permanent |
| `fkst-session-retired` | still-open work issues | the trigger was closed, the session retired and its pod cleaned up. These issues stay open but are no longer worked. Start a replacement session and assign the issue to that session's creator | permanent |

### The comment lead strings to recognize in a thread

These are the exact leads the platform writes, so they can be recognized when a user
pastes or describes an issue thread:

- **🟢 **fkst session `name` registered.**** — the registration comment; ends with
  "🔒 Config frozen."
- **👀 **Picked up by fkst session `name`.**** — the session claimed a work issue.
- **⚠️ fkst can't run this trigger issue as a session: {reason}** — paired with
  `fkst-substrate-invalid`.
- **🚫 **This trigger issue was not accepted: {detail}.**** — paired with
  `fkst-trigger-unauthorized`.
- **⚠️ **This issue carries an fkst work label but is not routed to any session.**** —
  paired with `fkst-unrouted`.
- **🚫 **@author is not authorized to raise work for fkst session `name`.**** — paired
  with `fkst-unauthorized`.
- **⚠️ **Session health: degraded**** — paired with `fkst-degraded`; followed by
  **✅ **Session health: recovered**** when it clears.
- **⚠️ **Session retired.**** — paired with `fkst-session-retired` on still-open work
  issues.

## Lifecycle — idle sleep versus a permanent stop

- **Closing the trigger issue is a permanent stop.** The session retires and never
  revives; a closed trigger is never re-registered. Every still-open work issue gets
  the retire notice and `fkst-session-retired`, and stays open. The dashboard **Stop**
  button does exactly this, behind a confirm dialog.
- **Trigger open with no open work means sleep.** After roughly 5 minutes with no open
  work-label issues the session's runtime is released. Sessions with no environment or
  with a reusable profile **auto-revive within about 30 s** when matching work appears —
  the same session, the same config.
- **A disposable environment is single-runtime.** Its private payload is consumed once
  the first sandbox accepts it. If that runtime is later released, lost or recreated,
  the control plane cannot reconstruct the environment, so the session blocks closed:
  close the trigger and create a new session.

## Environments — reusable profiles and disposable one-time setups

Exactly one environment mode per session: none, one reusable profile, or one disposable
environment. A request supplying both `environment` and `disposable_environment` is
rejected with `400` before GitHub is written.

### Reusable environment profiles

A named, reusable profile of ordered install commands, plain variables, and
**write-only secrets**, selected by a trigger's `### Environment`. Manage them in the
dashboard (top bar → **Environments**) or via
`GET /api/v1/users/me/environment-profiles` to list and
`GET`/`PUT`/`DELETE /api/v1/users/me/environment-profiles/{name}` for one profile, with
an `Authorization: Bearer <github-token>` header.

- **Name**: lowercase letters, digits, inner hyphens; ≤ 40 characters; fixed after
  creation.
- **Saving validates first**: the install commands run in an isolated throwaway runtime
  before anything persists, so a save takes a while. If a command fails nothing is
  saved, and the error carries the failing command, its exit code, and a stderr tail.
- **Secrets are write-only**: values are never shown or returned after saving. When
  editing, re-enter every secret value — a secret left blank is removed.
- Deleting a profile means sessions referencing it will no longer find it.
- Profiles belong to the user who saved them: a trigger's `### Environment` must name a
  profile that trigger's author has saved and validated.
- There is a per-user cap on profile count; the error states the limit.

Deployment limits (defaults): **50 install commands**, **4,096 bytes per command**, 100
entries in each variable/secret map, and 65,536 bytes per value. Keys must match
`^[A-Za-z_][A-Za-z0-9_]*$`, must not be platform-reserved, and cannot appear in both
the variables and secrets maps.

### Disposable one-time environments

Supplied only through the dashboard's **New session** dialog or the authenticated
create-session API — never authored in a GitHub issue. It has no name and is never
saved as a profile. Its object accepts any combination of `install` (ordered shell
commands run once in the real session sandbox before the engine), `variables`
(non-secret process environment variables) and `secrets` (write-only). At least one
entry is required. Unlike a saved profile, these commands are **not** pre-validated, so
they must be verified before submission.

The trigger issue records only this fixed marker under `### Environment`:

```
Disposable one-time environment. Details are injected privately into the session sandbox and are not stored in this GitHub issue.
```

No submitted command, variable or secret is ever written to the issue, to comments, or
to durable storage; the `201` response never contains the object, and logs record counts
only. The payload lives in process memory, bound to the verified creator, until the
sandbox accepts the complete bundle. There is deliberately no read or update endpoint
for it: recovery was traded away for privacy. Putting the marker in an issue by hand
also fails, because no private handoff exists — and the missing details must never be
pasted into the issue to work around that.

## Session logs

Every session auto-streams **redacted** logs. The 📥 Logs URL in the registration
comment is `…/api/v1/logs/{session_id}`. Fresh content lands roughly every 20 seconds
while a session runs, so what is fetched is typically one flush behind live.

**Access is deny-by-default.** It is granted to the trigger author, anyone on the
trigger's `### FKST Contributors` list (alias `### Log Access Allowlist`), or a
deployment administrator. Anyone else gets `403` — which is a correct answer about that
user's access, not a platform fault.

Two ways in:

- **Browser** — open the URL; it round-trips through GitHub sign-in and the redacted
  `.tar.gz` downloads. The browser flow always serves the latest bundle.
- **API** — `GET /api/v1/logs/{session_id}` with `Authorization: Bearer <github-token>`
  (any valid token; it only establishes identity). The redacted `.tar.gz` streams back.

**Per-run bundles**: each incarnation of a session is a **run** with its own immutable
bundle.

- `GET /api/v1/logs/{session_id}/runs` lists runs newest-first as
  `{run_id, started_at, ended_at}`; `ended_at` is absent while a run is live.
- Add `?run=<run_id>` to fetch that run; omit it (or use `run=latest`) for the newest.
- The dashboard's **Logs** tab has a run picker plus an in-browser viewer (file tabs,
  tail view, in-file search, full-file load, download link) — usually the fastest way to
  read logs.

For scripted peeking without downloading a whole bundle:
`GET /api/v1/logs/{session_id}/manifest?run=…` lists the bundle's files, and
`GET /api/v1/logs/{session_id}/file?path=<exact path>&tail_bytes=<N>&run=…` returns one
file or just its tail.

**Redaction** happens *before* storage: known session secrets, credential-shaped strings
(tokens, API keys, private keys, JWTs, passwords, auth headers) and high-entropy runs
are masked as `«REDACTED:…»`. A bundle is safe to share with an authorized user, but is
still session-sensitive.

## Observe a session's live engine state

`GET /api/v1/sessions/{session_id}/observe?limit=N` (Bearer token, same access rule as
logs) returns a live snapshot of the session's work queues as JSON: per-queue depth,
in-flight and retrying counts, dead-letter records, and run records. It never contains
message content. `limit` caps the delivery entries (default 500, clamped to 1–10000).

- `404` — unknown session, **or** the session is currently asleep with no live runtime.
- `409` — this session's packages have no durable delivery store to observe. Nothing to
  show; not an error.

The dashboard's Status tab surfaces the same snapshot as "Live engine details" while a
session is running.

## The dashboard, and who may do what

Sign in with **Sign in with GitHub**. The token refreshes automatically; if a session
expires the user keeps their place and signs in again. An EN / 中文 toggle and a guided
tour (the **?** button) live in the top bar.

- **Accounts view** — one card per reachable GitHub account (personal and
  organizations) with per-repository status dots: grey = App not installed, lit =
  installed, blinking = active sessions. Click to zoom in; breadcrumbs or Esc go back.
- **Repositories view** — the account's repositories with install state and package
  counts.
- **Repository workspace** — the session rail (auto-refreshing every 15 s) plus a detail
  pane with four tabs: **Status** (progress, lifecycle phase and health, a timeline,
  work items with state chips, live engine details), **Packages** (the frozen
  registration and every package reference with copy buttons), **Logs** (run picker,
  viewer, bundle download) and **Outcomes** (the session's pull requests with per-file
  diffs and inline previews).
- **Manage repositories and the App** — create a repository, install the App per account
  or per repository, manage an installation on GitHub, or uninstall it (behind a confirm
  dialog; everything the App covers in that account stops immediately).
- **Broader visibility (optional)** — by default the dashboard lists accounts and
  repositories where the App is installed. A dismissible banner offers **Connect**: a
  second, read-only GitHub authorization that also lists repositories and organizations
  where the App is *not* installed, useful for planning installs. It affects only what
  the overview lists, lasts for the browser tab session, and changes nothing about the
  login or the sessions.

### Per-action authority

- **Stop session** — the session's trigger **author**, or a **repository admin / org
  owner**. Session Collaborators deliberately cannot stop a session.
- **Add work item** — the trigger **author**, a listed `### Session Collaborators`
  login, or a **repository admin / org owner**.
- **New session** — a repository `admin` or `maintain` user, or a verified global admin.
  The dialog pre-checks work-label collisions and rejects one with the conflicting
  session's issue number.

## Deployment access and the admin model

A deployment's operator configures who may use it at all. The models are: every
authenticated GitHub user; an **allowlist** (only listed users plus global admins); or a
**denylist** (every authenticated user except the blocked ones, with global admins
always passing).

If an account is not permitted, every dashboard and API call answers `403 — this
deployment restricts access`, and that user's trigger issues are silently ignored: no
comment, no label, no session. A **blocked** user loses every gate — `403` on
token-authenticated routes, trigger issues ignored, work authorship denied in every
tier, and running sessions torn down on the next reconcile.

Deployments may also configure **global admins**. A verified global admin always passes
the service and trigger gates, is an authority tier for every session's work issues,
and has a dashboard spanning every account and repository where the deployment's App is
installed, including cross-installation session, outcome, log and observe read access.
Cross-account GitHub mutations still use the caller's own token and remain subject to
GitHub's permissions.

This is operator configuration: the honest answer to "am I allowed?" is what the API
returns for that user, not a guess.

## REST API quick reference

All paths are relative to the deployment's own origin. Dashboard requests use
`Authorization: Bearer <github-token>`; the server verifies the token's GitHub identity
and the deployment access policy.

| Method + path | Purpose |
|---|---|
| `GET /openapi.json` | the full, always-current API contract (no auth) |
| `GET /health` | liveness probe |
| `GET /api/v1/overview` | every visible account and repository with install status and live session/package counts |
| `GET /api/v1/repos/{owner}/{name}/sessions` | one repository's sessions (triggers, work issues, PRs, liveness) |
| `POST /api/v1/repos/{owner}/{name}/sessions` | create a trigger as the caller |
| `DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}` | stop a session (closes the trigger) |
| `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items` | queue a work item |
| `GET /api/v1/repos/{owner}/{name}/sessions/{issue_number}/outcomes` | the session's pull requests and their files |
| `GET /api/v1/sessions/{session_id}/observe` | live engine read-model |
| `GET /api/v1/logs/{session_id}` | download the redacted log bundle |
| `GET /api/v1/logs/{session_id}/runs` | list a session's runs |
| `GET /api/v1/logs/{session_id}/manifest` | list one run bundle's files |
| `GET /api/v1/logs/{session_id}/file` | read one log file (or its tail) |
| `GET /api/v1/users/me/environment-profiles` | list the caller's environment profiles |
| `GET`/`PUT`/`DELETE /api/v1/users/me/environment-profiles/{name}` | one profile |

Creating a session additionally requires repository `admin` or `maintain` permission
unless the caller is a verified global admin.

The create-session body fields are: `name`, `packages`, `manifests`, `work_label`,
`environment`, `disposable_environment`, `source_branch`, `target_branch`, `auto_merge`,
`log_access`, `collaborators`, `output_lang`. Success is `201` with
`{"issue_number": 123, "html_url": "…"}`; errors use
`{"error": "<stable_code>", "message": "<client-safe text>"}`.

| Status | Meaning |
|---|---|
| `400` | malformed or inconsistent fields, parser round-trip failure, or GitHub rejected the issue |
| `401` | missing or invalid GitHub token |
| `403` | deployment access denied, insufficient repository role, or GitHub refused the write |
| `404` | repository absent or invisible to the caller |
| `409` | an explicit work label collides with another open session |
| `422` | invalid disposable contents or branches, disabled issues, or another semantic precondition |
| `503` | GitHub unavailable, or this process cannot provide the disposable handoff |

The session API does not expose `### Engine Config` — author a GitHub trigger issue for
those advanced settings. There is no disposable-environment read or update endpoint.

## Troubleshooting quick answers

**"My trigger issue was ignored for minutes."** The webhook path is probably down, so
only the periodic resync catches up: a work issue on a registered session waits for the
~30 s sweep, and a brand-new trigger on an unregistered repository waits for the full
resync (~10 minutes). Also confirm the issue carries `fkst-substrate-trigger` and that
the account is permitted by the deployment.

**"It says `fkst-trigger-unauthorized`."** The effective creator is not attributable or
lacks authority. A human-authored trigger uses its author; a bot-authored trigger needs
**exactly one** assignee. Assign exactly the intended creator and ensure they are a
global admin or hold repository admin/maintain permission. The body is not parsed until
this clears — and it clears by itself once fixed.

**"It says `fkst-substrate-invalid`."** Read the ⚠️ comment: it names the exact problem
(parse error, duplicate heading, unreachable refs, no work label, colliding label, a bad
Engine Config key). Fix the body; the label clears on the next sweep. A named
environment that does not exist yet does *not* flag the trigger — the bot comments that
the session could not start and retries once the environment exists.

**"My work issue says `fkst-unrouted`."** It must have **exactly one assignee**, and
that assignee must be the creator of an active session watching one of its labels.
Remove extra or wrong assignees and assign the matching creator.

**"My work issue says `fkst-unauthorized`."** The author is not the session's creator, a
Session Collaborator, or a global admin. Authorization is always enforced. Once the
author becomes authorized the latch clears and the issue is picked up.

**"I edited the trigger and nothing changed."** Configuration froze at registration
(the 🟢 comment). The edit was rejected and `fkst-config-rejected` recorded. Close the
trigger and open a new one.

**"The session made a PR but never started the next task."** An open work-label issue is
what keeps a session alive; an unmerged PR does not idle it. Check whether the next task
issue exists, carries a work label, and has exactly one correct assignee.

**"The session disappeared."** If the trigger issue was closed the session retired
permanently and cannot revive. If the trigger is still open with no open work, the
session is merely asleep and revives within ~30 s when matching work appears — unless it
used a disposable environment, which is single-runtime.

**"I can't download the logs."** Log access is deny-by-default: the trigger author,
`### FKST Contributors` entries, and deployment administrators only. A `403` means that
account is not on the list; access lists cannot be widened after registration.

**Rules of thumb worth repeating.** Wave the backlog by dependency — land foundational
work issues, **merge them**, then open the issues that build on them; a dependent task
worked before its foundation merges can yield an empty diff or reference files that do
not exist yet (dependency ordering, not wording, is the usual failure mode). One feature
per work issue. Never put secrets, tokens, commands, or environment values in an issue.
Auto-merge bypasses review, so enable it only where that is acceptable. Give the
platform a sweep before concluding something is broken.
