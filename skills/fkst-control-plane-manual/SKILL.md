---
name: fkst-control-plane-manual
description: >-
  Operator manual for the fkst-hosted control plane — how to start, drive, monitor, and
  stop autonomous fkst-substrate coding sessions ENTIRELY through GitHub issues (there is
  no other API surface for this). Use whenever you need to: (a) launch a session by opening
  an `fkst-substrate-trigger` issue with the exact `### Session Name` / `### Packages` /
  `### Work Label` / optional `### Environment` / `### Auto-merge` / `### Log Access Allowlist` / `### Output Language` / `### Engine Config` body;
  (b) queue tasks by opening issues that carry the session's Work Label; (c) interpret the
  session's status comments and labels (registered / picked-up / degraded / retired /
  invalid / config-rejected); (d) download a session's redacted logs from the identity-gated
  `/api/v1/logs/{session_id}` endpoint; or (e) stop or idle a session by closing issues.
  Covers the package-reference grammar `owner/repo@ref:path`, config immutability, the
  idle-vs-permanent lifecycle, and the one-work-label-per-trigger rule.
version: "0.1"
homepage: https://github.com/ChronoAIProject/fkst-hosted
user-invocable: /fkst-control-plane-manual
metadata:
  category: plain
  tag:
    - fkst-hosted
    - control-plane
    - substrate
    - github-issues
    - manual
lastUpdated: 2026-07-03
---

# fkst-hosted control plane — operator manual

**fkst-hosted** is a GitHub-App-driven control plane on Kubernetes that runs autonomous
**fkst-substrate** coding sessions. There is **no dashboard and no REST API you drive by
hand** — the entire control surface is **GitHub issues**. You start a session by opening an
issue, queue work by opening more issues, watch progress through the comments/labels the
control plane writes back, and stop it by closing issues. The control plane reconciles the
declared state (open trigger issues) toward reality (one Kubernetes pod per live session).

## Mental model

| Concept | Is a… | You control it by… |
|---|---|---|
| **Session** | one long-lived substrate agent (one K8s pod) | opening/closing a **trigger issue** |
| **Trigger issue** | the session's declaration (its name, packages, work label) | an issue labeled `fkst-substrate-trigger` |
| **Work item** | one task for the session to pick up | an issue carrying the session's **Work Label** |
| **Session id** | stable UUID = `derive_session_id(installation, owner, repo, trigger#)` | derived — stable across idle→revive |

One trigger issue ⇒ one session. Open work-label issues ⇒ the queue that session works,
each as its own PR. The reconciler polls open issues, so actions land within a sweep
(seconds), not instantly.

## 1. Start a session — open a trigger issue

Open a GitHub issue **labeled `fkst-substrate-trigger`** whose **body** has these `### `
sections (matched by exact heading; a duplicate `### ` heading → the issue is flagged
invalid). Intro text before the first heading is ignored.

| Section | Required | Rule |
|---|---|---|
| `### Session Name` | **yes** | exactly one non-empty line; DNS-label-ish (`^[a-z0-9]([-a-z0-9]*[a-z0-9])?$`-style, ≤ the env-name cap) so it composes into K8s object names |
| `### Packages` | **yes** | ≥1 line, each a fully-qualified ref `owner/repo@ref:path` (see grammar below) |
| `### Work Label` | **yes** | exactly one non-empty line; a valid GitHub label, **≤ 50 chars, no comma** (the substrate reads it from a comma-separated env var) |
| `### Environment` | optional | one pre-provisioned environment name to inject (or blank → none). **Never put secret values here** — this only *selects* a profile provisioned out-of-band |
| `### Auto-merge` | optional | `true`/`yes`/`on`/`enabled`/`1` (case-insensitive) → the App bot's PRs are auto-merged into the default branch and the linked work issue auto-closed; anything else → off |
| `### Log Access Allowlist` | optional | extra GitHub logins/numeric-ids (beyond the author + global admins) allowed to download this session's logs; whitespace/comma/newline separated; a leading `@` is stripped. **Frozen at registration** (see config immutability) |
| `### Output Language` | optional | one locale tag (`^[a-z]{2,3}([-_][A-Za-z0-9]{2,8})?$`, e.g. `en`, `zh`, `zh-CN`) → the session's packages emit user-visible prose (issue/PR comments) in that locale via `FKST_OUTPUT_LANG`. The value must **exactly** match a `locales/<value>.lua` file shipped by the session's package (case- and separator-sensitive); a mismatch silently falls back to English. Absent/blank → English |
| `### Engine Config` | optional | advanced engine tunables, one `KEY=value` per line from a strict allowlist: `FKST_CODEX_PERMIT_SLOTS` (1..32), `FKST_QUEUE_CAPACITY` / `FKST_MAX_IN_FLIGHT_PER_DEPT` / `FKST_DURABLE_ADMISSION_BURST_PER_DEPT` (1..1024), `FKST_RETRY_DEFAULT_MAX_ATTEMPTS` (1..100), the four duration keys (`<n>s\|m\|h`, 1s..7d normalized; effective retry `CAP >= BASE`, defaults 60s/30m), and `FKST_RATE_POOL_<NAME>=<burst>,<refill/min>` (platform pool defaults can only be TIGHTENED, never widened). Any other key → `fkst-substrate-invalid` with the rule in the comment

### Package reference grammar — `owner/repo@ref:path`
Split greedily on the **first `@`** (`owner/repo` vs `ref:path`), then the **first `:`**
(`ref` vs `path`).
- `owner`, `repo`: `^[A-Za-z0-9_.-]+$`, exactly one `/` between them.
- `ref`: branch/tag/SHA, `^[A-Za-z0-9_./-]+$`, no `..` segment.
- `path`: repo-relative, `^[A-Za-z0-9_./-]+$`, not absolute, no `..` segment.

Example — a devloop session against `ChronoAIProject/fkst-packages@dev`:
```
### Session Name
sitebuilder

### Packages
ChronoAIProject/fkst-packages@dev:packages/github-devloop
ChronoAIProject/fkst-packages@dev:packages/github-devloop-pr
ChronoAIProject/fkst-packages@dev:packages/github-devloop-ops
ChronoAIProject/fkst-packages@dev:packages/consensus

### Work Label
site-build

### Auto-merge
true
```
Create it with the CLI:
`gh issue create --repo <owner>/<repo> --title "[session] sitebuilder" --body-file body.md --label fkst-substrate-trigger`

> ⚠️ The App must be **installed on the repo** and able to **reach every package ref**
> (public, or in a repo the App can read). An unreachable ref or a malformed body makes the
> reconciler flag the trigger `fkst-substrate-invalid` with a comment explaining the fix —
> fix the body and the flag clears on the next sweep.

## 2. Queue work — open Work-Label issues

Open one issue **per task**, labeled with the session's **Work Label**. Give each a clear
title, the **exact files** to change, real acceptance criteria, and enough spec to be worked
in isolation (the agent sees that one issue + the repo, not the sibling backlog). The
session picks them up, opens a PR per issue, and (if `Auto-merge` is on) merges + closes them.

## 3. Read the status the control plane writes back

| Signal | Where | Meaning |
|---|---|---|
| 🟢 `session … registered` comment | trigger issue | session accepted; includes the **📥 Logs:** URL and a hidden config-hash marker |
| 👀 pick-up comment | work issue | the session claimed this work item |
| PR by the App bot | repo PRs | the session's output for a work item |
| `fkst-degraded` label + comment | trigger issue | the pod looks unhealthy (crash/restart or recurring framework error); cleared when it reads healthy again |
| `fkst-session-retired` label + comment | still-open work issues | the trigger was closed → session retired; the item is no longer worked |
| `fkst-substrate-invalid` label + comment | trigger issue | the body failed to parse or a package is unreachable — fix and it clears |
| `fkst-config-rejected` label + comment | trigger issue | you edited the config of an already-registered session (see immutability) |

### Label glossary
`fkst-substrate-trigger` (you apply, to declare a session) · **your Work Label** (you apply,
to queue work) · `fkst-substrate-active` (latched once announced) · `fkst-picked-up` (latched
on a claimed work issue) · `fkst-session-retired` · `fkst-degraded` · `fkst-substrate-invalid`
· `fkst-config-rejected`. The `fkst-*` status labels are managed by the control plane — you
don't set them.

## 4. Lifecycle — idle vs. permanent

- **Close the trigger issue = permanent stop.** The session is retired, the pod cleaned up,
  and it will **never revive** (a closed trigger is never re-registered). Still-open work
  issues get `fkst-session-retired` + a comment.
- **Trigger open + no open work = idle.** The pod is killed to save resources, but the
  session **auto-revives** (re-spawns) the moment a new matching work issue appears — no new
  trigger needed. So: to pause, close/merge all work; to resume, open a work issue.
- **An open work issue keeps the pod alive.** Merge/close finished work to let a session idle
  down.

## 5. Config immutability

Once a session has registered, its config (`### Packages` / `### Work Label` / `### Environment`
/ `### Auto-merge` / `### Log Access Allowlist` / `### Output Language` / `### Engine Config`) is **frozen**. Editing the trigger body does **not**
re-launch — the control plane posts a one-time `fkst-config-rejected` comment. **To change
config, close the trigger and open a new one.** (This is why `### Log Access Allowlist` can't be widened
after the fact to grant retroactive log access.)

## 6. Download a session's logs

Every session **auto-streams its redacted logs** to storage; the 📥 Logs URL in the
registration comment is `…/api/v1/logs/{session_id}`. Access is **identity-gated, deny by
default**, authorized if the requester is the **trigger author**, on the **`### Log Access Allowlist`
allow-list**, or a **global admin**. Two ways in:
- **Browser** — just open the URL; it redirects through GitHub login, then the redacted
  `.tar.gz` downloads. (No S3 URL is ever exposed; the control plane streams the bytes.)
- **Agent/API** — `GET …/api/v1/logs/{session_id}` with `Authorization: Bearer <github-token>`;
  the token is traded for your identity and the redacted `.tar.gz` streams back.

Logs are the latest flush (refreshed every ~20 s / 256 KB and on pod exit) and are
**redacted** (secrets masked) — safe to share with an authorized user, but treat them as
session-sensitive.

## Rules of thumb (learned the hard way)

- **One Work Label per open trigger, per repo.** Two open triggers sharing a label spawn
  competing pods over the same queue (double-claim / duplicate PRs).
- **Wave the backlog by dependency.** Land foundational work issues, **merge them**, *then*
  open the issues that build on them. A dependent issue worked before its foundation merges
  can yield an empty diff or reference files not yet on the branch. Dependency ordering — not
  wording — is the usual failure mode.
- **One feature per work issue**, named in the title, with exact files + checkable acceptance
  criteria.
- **Never put secrets, tokens, or env *values* in an issue.** Use `### Environment` to *select*
  a pre-provisioned profile; values are supplied out-of-band, never read from the issue.
- **Give it a sweep.** Actions are reconciled on a poll; expect seconds, and re-check the
  issue's comments/labels rather than expecting instant effect.
