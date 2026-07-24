---
name: fkst-control-plane-manual
description: >-
  Operator and user manual for fkst-hosted autonomous coding sessions. Use when Codex
  needs to create, drive, inspect, or stop a session through GitHub issues, the dashboard,
  or the REST API; choose or manage reusable environment profiles; submit a private
  disposable one-time environment; queue work; interpret platform labels and comments;
  inspect logs, runs, outcomes, or live engine state; manage GitHub App installations;
  or reason about authorization, immutable configuration, lifecycle, recovery, package
  and manifest references, work-label discovery, and collisions.
metadata:
  category: plain
  tag:
    - fkst-hosted
    - control-plane
    - substrate
    - github-issues
    - dashboard
    - api
    - manual
---

# fkst-hosted — user manual

**fkst-hosted** runs long-lived autonomous coding sessions anchored by GitHub issues:
create a trigger to start a session, queue tasks as more issues, and each task
comes back as its own pull request. You have **three control surfaces**:

- **GitHub issues** — the durable source of truth for session declarations, work queues,
  status, and stopping. Do not put disposable environment details in an issue.
- **The web dashboard** — a visual companion for the same operations: sign in with GitHub
  to see every account, repository, and session; start sessions, queue work, stop sessions,
  manage environments and App installations, and read logs/outcomes without leaving the
  browser.
- **The REST API** — the machine-readable dashboard surface under `/api/v1/*`. In
  particular, `POST /api/v1/repos/{owner}/{name}/sessions` is the only non-UI way to submit
  a disposable environment privately while the API creates the corresponding trigger
  issue as the authenticated user.

The trigger issue remains the durable declaration even when the dashboard or API creates
it. A disposable environment is the deliberate exception to issue-only state: its private
details exist only in a transient, process-local handoff until the sandbox accepts them;
the issue records a fixed, non-sensitive marker instead.

Actions take effect on a short platform sweep — expect **seconds (~30 s)**, not instant
feedback; a brand-new repository can take up to ~10 minutes to be noticed unless webhook
delivery is active. Re-check the issue's comments and labels rather than assuming an
immediate effect.

## Mental model

| Concept | Is a… | You control it by… |
|---|---|---|
| **Session** | one long-lived coding agent bound to a repository | opening/closing a **trigger issue** (or dashboard New session / Stop) |
| **Trigger issue** | the session's declaration: name, packages/manifests, labels, options | an issue labeled `fkst-substrate-trigger` |
| **Work item** | one task for the session — it becomes a pull request | an issue carrying one of the session's **work labels** (or dashboard "Add work item") |
| **Run** | one incarnation of a session's runtime (sessions sleep and revive) | nothing — but each run keeps its own downloadable log bundle |

One trigger issue ⇒ one session. Open work-label issues ⇒ the queue that session works,
each as its own PR. A session **runs only while it has open work**: with no open work-label
issues it goes to sleep after ~5 minutes of idle grace. Sessions with no environment or a
reusable profile auto-revive when matching work appears; disposable sessions cannot revive
after their first runtime is released (see §6–§7).

## 1. Install the GitHub App

Install the fkst-hosted GitHub App on the repositories where sessions should run — from the
dashboard's **Install the GitHub App / Connect / Install** buttons (they deep-link to
GitHub's install page) or directly on GitHub. Notes:

- If you are not an admin of the target repository, GitHub may route your install as an
  approval request to its owner.
- **Auto-seeded starter session**: on install (when enabled, the default), each repository
  with no open trigger issue gets one auto-created trigger issue —
  `[session] default-workflows (auto-seeded)` — that references the platform's default
  workflow **manifest**, has **Auto-merge on**, and lists the installing account as an FKST
  Contributor. It behaves like any other trigger: its config freezes once registered, and
  closing it retires the session. Delete/close it if you don't want the default workflows.
- The App also installs two issue templates into the repository (kept up to date
  automatically): **"fkst substrate session"** (pre-labeled trigger scaffold) and
  **"fkst work item"** (task scaffold). Blank issues are disabled in favor of these.

## 2. Start a session — the trigger issue

Open a GitHub issue **labeled `fkst-substrate-trigger`** (the "fkst substrate session"
template applies the label for you). The **body** is a set of `### ` sections, matched by
exact heading. Text before the first heading is ignored; a **duplicate heading makes the
issue invalid**; `#### ` and deeper are ordinary text. HTML comments (`<!-- … -->`) in the
template are ignored, so you can fill the template in place.

| Section | Required? | Rule |
|---|---|---|
| `### Session Name` | **yes** | exactly one line: lowercase letters, digits, and inner dashes (`my-session`), 1–40 chars |
| `### Packages` | one of these two | zero or more lines, each a package reference `owner/repo@ref:path` (grammar below) |
| `### Manifest` | one of these two | zero or more lines, each a **manifest reference** `owner/repo@ref:path` pointing at an fkst-manifest JSON file |
| `### Work Label` | optional | **exactly one** label, ≤ 50 chars, **no comma**. Omit (or leave blank) to auto-discover labels from the session's packages |
| `### Environment` | optional | the name of **one** reusable environment profile you have saved (see §7). Selects the profile only — **never put commands, variable values, or secrets in an issue**. Disposable environments cannot be authored directly in a GitHub issue |
| `### Source Branch` | optional | branch used to seed a missing target branch; omit to use the repository's default branch |
| `### Target Branch` | optional | branch the session works against; omit to use `fkst-hosted-default` |
| `### Auto-merge` | optional | `true` / `yes` / `on` / `enabled` / `1` (case-insensitive) turns it on: the App bot's PRs auto-merge to the default branch when mergeable and the linked work issue is closed. Anything else = off. Note this bypasses your review/checks flow |
| `### FKST Contributors` | optional | the session's **trusted users** (you, the author, are always included): the session acts only on issues/comments from these people, and they may download the session's logs. GitHub logins and/or numeric ids, separated by spaces/commas/newlines; a leading `@` is fine; numeric ids count for log access only. Legacy heading `### Log Access Allowlist` is a permanent alias (both may appear; lists merge) |
| `### Session Collaborators` | optional | people granted **work-item authority** — they may raise, label, and comment on this session's work issues (distinct from log access; they cannot stop the session). Same list format |
| `### Output Language` | optional | one locale tag (`en`, `zh`, `zh-CN`, …). It must exactly match a locale the session's package ships, or output silently falls back to English |
| `### Engine Config` | optional | advanced tunables, one `KEY=value` per line from a strict allowlist (below). Any other key makes the trigger invalid |

**At least one package source is required**: a trigger with neither `### Packages` nor
`### Manifest` is invalid ("the trigger must list at least one package source").

### Package and manifest references — `owner/repo@ref:path`

Both sections use the same grammar. A reference splits at the first `@` (into `owner/repo`
and the rest) then the first `:` (into `ref` and `path`):

- `owner`, `repo` — letters, digits, `.`, `_`, `-`; exactly one `/` between them.
- `ref` — a branch, tag, or commit SHA; no `..` segments.
- `path` — repository-relative (never absolute, no `..`). For `### Packages` it points at a
  **package directory**; for `### Manifest` it points at the **manifest JSON file itself**.

Every referenced repository must be **public** and contain the expected content at that ref
and path — an unreachable reference blocks the session with a comment listing exactly which
refs failed and why.

### Manifests — bundles that expand into packages

A **manifest** is a JSON file (`{"schemaVersion": 1, "name": …, "packages": [ …refs… ]}`,
1–64 package refs) that the platform expands into its package list. A manifest is enough on
its own — a session can reference only a manifest. You may combine both sections: the
effective package set is your explicit `### Packages` lines first, then each manifest's
packages, de-duplicated (your explicit entry wins its position). A manifest that can't be
fetched or doesn't validate makes the trigger invalid — it never partially applies.

### Work labels — one explicit, many discovered

- The explicit `### Work Label` is still **at most one** label per trigger.
- Packages can declare their own work labels; the session's **effective label set** is your
  explicit label plus every label its packages declare. The registration comment lists the
  full set — the session picks up issues carrying **any** of them.
- If you omit `### Work Label` and the packages declare none, the trigger is flagged
  invalid: *"no work label: add a `### Work Label` section or use packages that declare
  work labels"*. Add a label (or use packages that declare labels) and the flag clears.
- **Collisions**: within one repository, each label belongs to the **oldest open trigger**
  (lowest issue number) that uses it. A newer trigger colliding on any of its labels is
  flagged invalid (*"work label 'x' collides with active session #N"*) until the older
  session closes or the label changes. The dashboard's New-session dialog pre-checks
  explicit-label collisions and warns before you submit.

### `### Engine Config` — allowlisted tunables

One `KEY=value` per line, no duplicates:

| Key | Accepted values |
|---|---|
| `FKST_LLM_MODEL` | a plain model id served by the deployment's LLM endpoint (letters, digits, `. _ / : -`; ≤ 128 chars) — runs THIS session on that model instead of the deployment default |
| `FKST_LLM_REASONING_EFFORT` | `minimal` \| `low` \| `medium` \| `high` \| `max` (case-insensitive, stored lowercase; deployment default `max`) |
| `FKST_CODEX_PERMIT_SLOTS` | integer 1–32 |
| `FKST_QUEUE_CAPACITY`, `FKST_MAX_IN_FLIGHT_PER_DEPT`, `FKST_DURABLE_ADMISSION_BURST_PER_DEPT` | integer 1–1024 |
| `FKST_RETRY_DEFAULT_MAX_ATTEMPTS` | integer 1–100 |
| `FKST_RETRY_DEFAULT_BASE`, `FKST_RETRY_DEFAULT_CAP`, `FKST_DEPARTMENT_DEFAULT_STALL_WINDOW`, `FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET` | a duration like `30s` / `5m` / `2h` (1 second – 7 days); the effective retry cap must stay ≥ the base (defaults 60s / 30m) |
| `FKST_RATE_POOL_<NAME>` | `<burst>,<refill_per_minute>`, both ≥ 1; `NAME` is `A–Z0–9_` (`ROOT` reserved). Platform pool defaults can only be **tightened**, never widened |

Do not set the output language here — use `### Output Language` (a dedicated error tells
you so).

### Example

```
### Session Name
sitebuilder

### Manifest
ChronoAIProject/fkst-packages@fkst-hosted:manifests/default-workflows.json

### Work Label
site-build

### Auto-merge
true
```

Create it from the CLI:
`gh issue create --repo <owner>/<repo> --title "[session] sitebuilder" --body-file body.md --label fkst-substrate-trigger`

Or use the dashboard: repository workspace → **New session** — the same fields as a form
(session name, package rows, manifests, work label with live collision warning, an
environment mode of None / Saved profile / Disposable, source and target branches,
auto-merge, log access allowlist, collaborators, output language). Disposable mode accepts
ordered install commands, variables, and masked secrets, then requires a second
confirmation that shows counts only. Back returns to the populated editor; confirm submits
one request. Check every value before confirming because the details cannot be updated.

### If something is wrong

Within a sweep the bot posts **one** comment — *"⚠️ fkst can't run this trigger issue as a
session: {reason}"* — and adds the `fkst-substrate-invalid` label. The reason names the
exact problem (parse error, unreachable refs, missing/colliding labels, bad Engine Config
key, …). **Fix the body and the label clears automatically** on the next sweep; no new
trigger needed. A named environment that doesn't exist (or isn't ready yet) doesn't flag
the trigger — the bot comments that the session couldn't start and retries once you've
created the environment and re-triggered.

## 3. Registration and config immutability

When the trigger is accepted, the bot posts the registration comment — **🟢 "fkst session
`name` registered."** — listing the session's **work label(s)** (the full effective set),
its packages, environment, auto-merge state, and (when configured) the **📥 Logs** URL. It
ends with **"🔒 Config frozen."** and the trigger gains the `fkst-substrate-active` label.

From that moment the session's entire configuration is **immutable**. Editing the trigger
body afterwards changes nothing: the running session keeps its original config, a one-time
comment explains the rejection, and the durable `fkst-config-rejected` label is added. **To
change anything, close the trigger and open a new one.** (Edits made *before* the 🟢
comment appears are fine. This is also why access lists can't be widened retroactively.)

For a disposable environment, the fixed marker is part of this frozen declaration, but its
private payload is never editable or recoverable from the issue. There is no read or update
API for that payload. Close the trigger and create a new session to correct any detail.

## 4. Queue work

Open one issue **per task**, carrying one of the session's work labels (the "fkst work
item" template gives you a scaffold; add the label to route it). Give each task a clear
title, the exact files to change, real acceptance criteria, and enough context to be worked
in isolation — the agent sees that one issue plus the repository, not the sibling backlog.
The session acknowledges the claim with a 👀 comment + the `fkst-picked-up` label, works
it, and opens a pull request for it (auto-merged and auto-closed if the session has
Auto-merge on).

From the dashboard: repository workspace → session card → **Add work item** (title +
optional Markdown details). This is available when the session has an explicit
`### Work Label`; auto-discovered-label sessions queue work by labeling issues on GitHub.

## 5. Status signals the platform writes back

You apply only `fkst-substrate-trigger` and your work labels — every other `fkst-*` label
is managed for you.

| Signal | Where | Meaning |
|---|---|---|
| 🟢 registration comment + `fkst-substrate-active` | trigger issue | session accepted; lists work label(s), packages, and the 📥 Logs URL; config now frozen |
| 👀 pick-up comment + `fkst-picked-up` | work issue | the session claimed this task |
| PR by the App bot | repository PRs | the session's output for a work item |
| `fkst-substrate-invalid` + ⚠️ comment | trigger issue | body invalid, refs unreachable, no work label, or label collision — fix it and the label auto-clears |
| `fkst-config-rejected` + ⚠️ comment | trigger issue | you edited a registered session's config; the edit is ignored (label is permanent) |
| `fkst-degraded` + ⚠️ comment | trigger issue | the session looks unhealthy; a ✅ "recovered" comment follows and the label clears when it's healthy again (checked every ~2.5 min) |
| `fkst-session-retired` + ⚠️ comment | still-open work issues | the trigger was closed → session retired; these issues stay open but are no longer worked |
| `fkst-unauthorized` + 🚫 comment | work issue | (only when the deployment enforces work-issue authority) the issue's author may not raise work for this session — it will not be picked up. Clears automatically if the author is later authorized |

## 6. Lifecycle — idle vs. permanent

- **Close the trigger issue = permanent stop.** The session retires and **never revives**
  (a closed trigger is never re-registered). Every still-open work issue gets the retire
  notice + `fkst-session-retired` and stays open. The dashboard **Stop** button does
  exactly this (with a confirm dialog).
- **Trigger open + no open work = sleep.** After ~5 minutes with no open work-label issues
  the session's runtime is released. Sessions with no environment or a reusable profile
  **auto-revive within ~30 s** when matching work appears — same session and config.
- **A disposable environment is single-runtime.** Its private payload is consumed after
  the first sandbox accepts it. If that runtime is later released, lost, or recreated, the
  control plane cannot reconstruct the environment without persisting it. The session
  blocks closed; close the trigger and create a new session.
- **An open work issue keeps the session alive.** Merge or close finished work to let it
  wind down.

## 7. Environment setup: reusable and disposable

Choose exactly one environment mode when creating a session: none, one reusable profile,
or one disposable environment. A request that supplies both `environment` and
`disposable_environment` is rejected with `400` before GitHub is written.

### Reusable environment profiles

An **environment** is a named, reusable profile — ordered install commands, plain
variables, and **write-only secrets** — that a session selects via `### Environment`.
Manage them in the dashboard: top bar → **Environments** (signed-in users), or from
scripts with `GET /api/v1/users/me/environment-profiles` to list and
`GET/PUT/DELETE /api/v1/users/me/environment-profiles/{name}` for one profile. Send an
`Authorization: Bearer <github-token>` header.

- **Name**: lowercase letters, digits, inner hyphens; ≤ 40 chars; fixed after creation.
- **Saving validates first**: your install commands are executed in an isolated, throwaway
  runtime before anything persists — expect the save to take a while. If a command fails,
  nothing is saved and you get the failing command, exit code, and a stderr tail.
- **Secrets are write-only**: values are never shown or returned again after saving. When
  editing, re-enter every secret value — secrets left blank are removed.
- Deleting a profile means sessions that reference it will no longer find it. There is a
  per-user cap on how many profiles you can keep (the error tells you the limit).
- Environments belong to **you** (the trigger author): a trigger's `### Environment` must
  name a profile the author has saved and validated.

### Disposable one-time environments

A disposable environment is supplied only through the dashboard's **New session** dialog
or the authenticated create-session API in §12. It has no name and is never saved as a
profile. Its request object contains any combination of:

- `install`: ordered shell commands run once in the real session sandbox before the engine;
- `variables`: non-secret process environment variables; and
- `secrets`: write-only secret process environment variables.

At least one command, variable, or secret is required. Commands are **not** pre-run in the
saved-profile validation sandbox, so verify them before submission. The deployment's
configured limits apply; defaults are 50 commands, 4,096 bytes per command, 100 entries in
each variable/secret map, and 65,536 bytes per value. Keys must match
`^[A-Za-z_][A-Za-z0-9_]*$`, must not be platform-reserved, and cannot appear in both maps.

The corresponding trigger issue contains only this fixed marker in `### Environment`:

```
Disposable one-time environment. Details are injected privately into the session sandbox and are not stored in this GitHub issue.
```

No submitted command, variable, or secret is written to the issue, comments, or durable
control-plane storage. The `201` response never contains the disposable object, and normal
diagnostic logs record counts only. A rejected `422` may name an invalid, reserved,
oversized, or overlapping environment key, but never returns its value or a submitted
command. The handoff is bound to the verified creator and retained in process memory only
until the selected sandbox backend accepts the complete bundle. A failed sandbox creation
keeps it available for retry; successful creation or closing the trigger erases it. OpenAPI
marks secret map names and values `writeOnly`; the object is request-only.

This privacy model deliberately trades away recovery. A control-plane restart or failover
before handoff loses the payload; a later runtime recreation cannot recover a consumed
payload. Both cases fail closed with a comment asking the user to close the trigger and
create a new session. Manually putting the marker in an issue also fails because no private
handoff exists. Never put the missing details into the issue to work around that failure.

## 8. Session logs

Every session **auto-streams redacted logs**; the 📥 Logs URL in the registration comment
is `…/api/v1/logs/{session_id}`. Fresh content lands roughly every 20 seconds while the
session runs (what you fetch is typically one flush behind live).

**Access is deny-by-default**, granted to: the **trigger author**, anyone on the trigger's
**`### FKST Contributors`** list (alias `### Log Access Allowlist`), or a **deployment
administrator**. Two ways in:

- **Browser** — open the URL; it round-trips through GitHub sign-in, then the redacted
  `.tar.gz` downloads. The browser flow always serves the **latest** bundle.
- **Agent / API** — `GET …/api/v1/logs/{session_id}` with
  `Authorization: Bearer <github-token>` (any valid token; it's only used to establish who
  you are). The redacted `.tar.gz` streams back.

**Per-run bundles**: each incarnation of the session (it sleeps and revives) is a **run**
with its own immutable bundle.

- `GET /api/v1/logs/{session_id}/runs` (Bearer token) lists runs newest-first as
  `{run_id, started_at, ended_at}` (`ended_at` absent while a run is live).
- Add `?run=<run_id>` to the download to fetch that run's bundle; omit it (or use
  `run=latest`) for the newest content.
- The dashboard's **Logs tab** has a run picker plus an in-browser viewer (file tabs,
  tail view, in-file search, full-file load, download link) — usually the fastest way to
  read logs.

For scripted peeking without downloading the whole bundle:
`GET /api/v1/logs/{session_id}/manifest?run=…` lists the bundle's files;
`GET /api/v1/logs/{session_id}/file?path=<exact path>&tail_bytes=<N>&run=…` returns one
file (or just its tail).

**Redaction**: bundles are redacted **before** they are stored — known session secrets,
credential-shaped strings (tokens, API keys, private keys, JWTs, passwords, auth headers),
and high-entropy runs are masked as `«REDACTED:…»`. Safe to share with an authorized user;
still treat them as session-sensitive.

## 9. Observe a session's live engine state

`GET /api/v1/sessions/{session_id}/observe?limit=N` (Bearer token; same access rule as
logs) returns a live snapshot of the session's work queues as JSON: per-queue depth /
in-flight / retrying, dead-letter records, and run records — never any message content.
`limit` caps the delivery entries (default 500, clamped to 1–10000).

- `404` — unknown session, **or** the session is currently asleep (no live runtime).
- `409` — this session's packages have no durable delivery store to observe (nothing to
  show; not an error).

The dashboard's Status tab surfaces the same snapshot as **"Live engine details"** while
the session is running.

## 10. The web dashboard

Sign in at the dashboard with **Sign in with GitHub**. You stay signed in (the token
refreshes automatically); if the session ever expires you keep your place and just sign in
again. An EN / 中文 language toggle and a guided tour (the **?** button) are in the top bar.

- **Accounts view** — one card per GitHub account you can reach (personal + organizations)
  with status dots per repository: grey = App not installed, lit = installed, blinking =
  active sessions. Click to zoom in; breadcrumbs / Esc go back; **Refresh** re-loads.
- **Repositories view** — the account's repositories with install state and package counts.
- **Repository workspace** — the session rail (auto-refreshes every 15 s) + a detail pane:
  - **Status** tab: progress, distribution, lifecycle phase and health, a timeline
    (session started → work items → PRs), the work-item list with state chips, and Live
    engine details.
  - **Packages** tab: the frozen registration (work label, environment, auto-merge, output
    language, manifests, access lists) and every package reference with copy buttons.
  - **Logs** tab: run picker + in-browser log viewer + bundle download.
  - **Outcomes** tab: the session's pull requests with per-file diffs and inline previews.
- **New session** — choose None, a reusable profile, or a disposable environment. The
  disposable editor accepts ordered commands, variables, and masked secrets and requires
  an irreversible-action confirmation that displays counts rather than private content.
- **Manage repositories & the App**: create a repository (owner picker, private by
  default), install the App per account or per repository (deep-links to GitHub),
  **Manage** an installation on GitHub, or **Uninstall** it (confirm dialog — everything
  the App covers in that account stops immediately).
- **Broader visibility (optional)**: by default the dashboard shows accounts/repositories
  where the App is installed. A dismissible banner offers **Connect** — a second, read-only
  GitHub authorization that also lists repositories and organizations where the App is
  *not* installed (useful for planning installs). Disconnect any time; it affects only
  what the overview lists, lasts for the browser tab session, and changes nothing about
  your login or the sessions.

### Who may do what (dashboard actions)

- **Stop session** — the session's trigger **author** or a **repository admin / org
  owner**. (Session Collaborators deliberately cannot stop a session.)
- **Add work item** — the trigger **author**, a listed **`### Session Collaborator`**, or
  a **repository admin / org owner**.
- **New session** — a repository `admin` or `maintain` user, or a verified global admin;
  the dialog pre-checks work-label collisions (a collision is rejected with the
  conflicting session's issue number).

These rules are always enforced for dashboard actions. On the GitHub side, work-issue
authority is additionally enforced only when the deployment turns that on — see the
`fkst-unauthorized` signal in §5.

## 11. Deployment access

A deployment may restrict who can use it at all (an operator-managed allowlist). If your
GitHub account isn't allowed: every dashboard/API call answers
`403 — this deployment restricts access; your GitHub account is not on the allowlist`, and
your trigger issues are **silently ignored** (no comment, no label, no session). Log
access additionally follows §8's per-session rule.

Deployments may configure `FKST_GLOBAL_ADMINS` with comma-separated GitHub logins
(or rename-safe numeric IDs). A verified global admin's dashboard spans every account
and repository where the deployment's GitHub App is installed, and the admin may read
the associated session details, outcomes, logs, and observe snapshots. The legacy
`FKST_LOG_ADMINS` list remains log/observe-only and does not widen the dashboard.

## 12. Machine-readable API

- `GET /openapi.json` — the full, always-current API contract (no auth required).
- `GET /health` — `{"status":"ok", …}` liveness probe.

Dashboard API requests use `Authorization: Bearer <github-token>`. The server verifies the
token's GitHub identity and the deployment allowlist. Creating a session additionally
requires repository `admin` or `maintain` permission unless the caller is a verified global
admin.

Before constructing, reviewing, or troubleshooting an API call, read
[references/api.md](references/api.md). It records the exact create-session request fields,
disposable schema and limits, response/error envelopes, status codes, fixed GitHub marker,
and replacement workflow. The primary write endpoints are:

- `POST /api/v1/repos/{owner}/{name}/sessions` — create a trigger as the caller;
- `DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}` — stop it; and
- `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items` — queue work.

The session API does not expose `### Engine Config`; author a GitHub trigger issue for
those advanced settings. There is no disposable-environment read or update endpoint.

## Rules of thumb (learned the hard way)

- **One explicit work label per trigger, and no sharing.** Labels are exclusive per
  repository — the oldest open trigger owns a label; newer claimants are flagged invalid
  until it's free.
- **Wave the backlog by dependency.** Land foundational work issues, **merge them**, *then*
  open the issues that build on them. A dependent task worked before its foundation merges
  can yield an empty diff or reference files that don't exist yet. Dependency ordering —
  not wording — is the usual failure mode.
- **One feature per work issue**, named in the title, with exact files and checkable
  acceptance criteria.
- **Never put secrets, tokens, commands, or environment values in an issue.** Use
  `### Environment` only to select a saved profile. Send one-time details through the
  dashboard/API `disposable_environment` body, and ensure the client does not log it.
- **Treat disposable as one runtime, not a recoverable profile.** Restart, failover, idle
  release, or pod loss can require a new trigger because the private payload is not stored.
- **Give it a sweep.** Actions land within seconds, not instantly — re-check the issue's
  comments and labels.
- **Auto-merge bypasses review.** The bot's PRs merge to your default branch when mergeable
  — only enable it on repositories where that's acceptable.
