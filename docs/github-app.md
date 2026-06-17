# GitHub App Integration: ADR and Operations Runbook

## ADR: Credential Division for fkst-hosted GitHub Access

### Status

Accepted (issue #59).

### Context

fkst-hosted needs GitHub credentials for three distinct purposes:

1. **Session credentials** -- the engine's git/API work on user repos during a fkst session (clone, push progress records, read/write files).
2. **User-attributed operations** -- creating repositories on behalf of users, managing issues in the user's name.
3. **Journaling** -- committing progress-record files to a dedicated journal repo (already served by the deploy-level `GITHUB_TOKEN`).

### Decision

We use three separate credential paths:

| Purpose | Credential | Scope | Attribution |
|---------|-----------|-------|-------------|
| Session work (git/REST on target repo) | GitHub App installation tokens (this module) | 1-hour, repo-scoped, permission-subsettable | fkst-hosted App identity (not the user) |
| User-attributed ops (create-repo, issues hub) | NyxID credential-injection proxy | User's OAuth token (fkst-hosted never sees it) | The user |
| Journaling | Deploy-level `GITHUB_TOKEN` | Dedicated journal repo | Service account |

### Rationale

**Why an owned GitHub App (not personal tokens)?**

- Installation tokens are short-lived (1 hour, non-refreshable, only re-mintable).
- They are scoped to exactly one installation (one org/user), and can be further scoped to specific repositories and a permission subset per mint.
- They are revocable: uninstalling the App instantly revokes all tokens.
- Per-installation rate limits avoid noisy-neighbor problems across tenants.
- fkst-hosted never holds users' raw GitHub OAuth tokens (NyxID does not expose token retrieval to downstream services).

**Why personal-account repo creation stays on the NyxID proxy path?**

- Installation tokens cannot create repositories under a personal account. That requires a user-attributed OAuth token, which NyxID proxies.
- This is precisely why both credential paths exist.

**Why are webhooks OFF in v1?**

- v1 uses only REST polling and git operations. No webhook receiver is deployed.
- Real-time issue sync (push events, pull request webhooks) is the v2 option.
- Keeping webhooks off reduces the attack surface and simplifies the initial deployment.

**Why is enterprise-level installation not supported?**

- GitHub Apps cannot be installed at the enterprise level as of this writing. The installation target is an organization or a user account.

### Consequences

- fkst-hosted operators must register and install the GitHub App manually (see runbook below).
- Until `FKST_GITHUB_APP_ID` is configured, the module is disabled (logged at boot) and dependent features return actionable 422 errors with an install URL.
- A bad PEM fails at deploy time (startup), not at first session.
- As of issue #110 the session permission set is **admin-equivalent** (see "Session token permissions" below); adding `administration` triggers per-installation re-approval and makes org installation owner-only. The owner has accepted the increased blast radius of an autonomous session holding repo-admin for the session duration.

### Session token permissions (issue #110)

Substrate sessions hold **admin-equivalent** access to their target repo for the
whole session. Every minted installation token requests these **Repository
permissions** at **write**:

| Permission | Level | Why |
|------------|-------|-----|
| **Administration** | Read & write | GitHub Apps have no "admin role"; `administration: write` is the closest equivalent — branch protection / rulesets, collaborator & team management, repo settings, visibility, rename/transfer, deploy keys. |
| **Pull requests** | Read & write | Open / update / merge PRs from session automation. |
| **Contents** | Read & write | Clone, push progress records, read/write session files. |
| **Issues** | Read & write | Create / comment / close issues from session automation. |
| **Metadata** | Read (implicit) | Always granted on installation tokens; never requested explicitly. |

The owner has explicitly accepted the increased blast radius of an autonomous
agent holding repo-admin for the session duration. The elevated scope only takes
real effect once the installation token is wired into the engine at startup
(issue #106).

#### Required GitHub App settings declaration (hard prerequisite)

A mint can only request a **subset of what the GitHub App itself was granted**.
The fkst-hosted GitHub App **must** declare all four of **Administration**,
**Pull requests**, **Contents**, and **Issues** as **Read & write** Repository
permissions in its settings. If the App was never granted one of them, the mint
returns **422** (`github token request rejected`) — this is logged loudly at the
mint site (`token_for_repo`) with the rejected-permission detail and surfaced to
callers as a 422; the token is never logged. Declare these in the App settings
**before** broad installation so new installs consent up-front (see runbook
step 1).

#### Re-consent requirement (existing installations)

Adding the `administration` permission to an already-published App **suspends
that permission on every existing installation until the owner re-approves**.
Existing installations get a review prompt; the elevated set does not take
effect on them until re-approved. Wire the operational signal through the
installation webhook/lifecycle (issue #108).

#### Org consequence: installation becomes owner-only

Requesting `administration` makes **organization installation owner-only**.
Repo admins are **excluded** from installing an App that requests the repository
Administration permission, and non-owner members can only **Request** it (an
org owner then approves). This changes the org install instructions from "an
admin installs" to "an **org owner** installs / approves".

#### Out of scope (deliberately NOT requested)

`workflows`, `secrets`, `actions`, `repository_hooks`, and `environments` are
**not** added by this change. Consequently the session token **cannot push
changes under `.github/workflows/**`** (that needs `workflows: write`) nor
**manage Actions secrets / variables** (needs `secrets`/`actions`). If a session
later needs those, file a follow-up issue rather than widening the default set
silently.

### Session-token delivery to the engine

A goal session's substrate engine receives its installation token **at t=0**, before the engine process is started (issue #106). The session driver mints the repo-scoped installation token and builds a `GoalContext`, then starts the engine via `start_with_spec(goal: Some(..))`. As a result, before the engine runs the driver has:

- written `<runtime_dir>/github-token` (mode `0600`) and `<runtime_dir>/goal.json`, and
- set `GITHUB_TOKEN`, `FKST_GITHUB_TOKEN_FILE`, and `FKST_GOAL_FILE` on the substrate child process.

The same `GoalContext` path is used identically by the initial start and by the failover rebuild on a takeover pod — the token is never persisted, always (re-)minted from the `SessionDoc`. Minting is a cache hit after the trigger-time install-check preflight (`token_for_repo` caches per `(repo, perms)`), so this is not a second network mint. The in-run periodic refresh then re-mints ~55 minutes later (5 minutes before the 60-minute TTL).

> Earlier (pre-#106) the driver started the engine with `goal: None` and the token only reached the runtime dir via the periodic refresh, which was suppressed for the first ~55 minutes — so the engine ran with no credential at startup. That regression is fixed: the token is present from t=0.

### Installation lifecycle: stateless resolution, webhook, and uninstall (#108, #141)

Installation resolution is **stateless** (#141): there is **no durable installation store** (the `github_installations` Mongo collection was removed as part of the database-free pivot). The App layer resolves on demand and caches in memory; staleness self-corrects at the next mint.

**Stateless resolve (on-demand probe + in-memory TTL cache).** `token_for_repo` / installation resolution checks, in order: (1) the **in-memory installation cache** (a jittered ~15-minute TTL, see below), then (2) the on-demand `GET /repos/{owner}/{repo}/installation` GitHub probe, whose result is cached so the next resolve is a cache hit. A cold pod (empty in-memory cache) probes on demand the first time it touches a repo. There is no database read on this path — a stale mapping (an install removed since the cache was warmed) self-corrects at the next mint: the mint's `InstallationGone` (403/404) backstop invalidates both caches and transparently re-resolves once. So the App-token path no longer depends on a database at all.

**TTL + jitter.** A cache entry's lifetime is `INSTALLATION_TTL_BASE` (15 min) plus a per-entry uniform random jitter in `0..=INSTALLATION_TTL_JITTER` (up to 5 min) — i.e. **15 min ± up to 5 min**, stored as an absolute `expires_at`. *Why jitter:* a fleet of N stateless workers that all cold-probe the same repo would otherwise expire and re-probe in lockstep, synchronising an N-wide stampede against the shared GitHub REST budget every TTL window. Spreading each entry's expiry across a ±5-minute window de-synchronises the refresh.

**Webhook endpoint (`POST /api/v1/github/app/webhook`) — a cache-bust hint.** UNAUTHENTICATED but signature-verified, mounted **outside** the `/api/v1` auth nest (like `/health`) and only when `FKST_GITHUB_APP_WEBHOOK_SECRET` is set. It performs **no durable persistence** — it is a hint that promptly evicts in-memory caches and fails affected sessions. The handler:

1. Reads the body as **raw bytes** and verifies the `X-Hub-Signature-256` HMAC-SHA256 over those exact bytes **before any JSON parse** (deserialize-then-reserialize changes the bytes and breaks the MAC). The compare is **constant-time** (`hmac::Mac::verify_slice`, no GitHub SDK — consistent with the hand-rolled `reqwest` + `jsonwebtoken` transport). A missing/malformed/mismatched signature is `401` with no detail.
2. Parses `X-GitHub-Event` **only to derive the affected `owner/name` set** (no per-event GitHub API call, no token mint). `installation` `deleted`/`suspend` evict + fail the enumerated `repositories` when present, else **account-wide** by the installation's `account.login`; `installation_repositories` evicts + fails only `repositories_removed` (`added` needs nothing — the next on-demand resolve picks it up). `created`/`unsuspend` are no-ops (nothing to bust). The handler is **idempotent** (GitHub redelivers) and returns `2xx` quickly; an internal processing failure (e.g. a malformed body) logs and answers `202` rather than triggering a redelivery storm.

**Uninstall → caches → sessions (cluster-wide).** On an `installation deleted` / `suspend` / `repository removed` event, the affected repos are evicted from the in-memory token + installation caches (so the next mint re-probes and correctly 404s), and any **active session** targeting an affected repo is transitioned to `Failed` with a clear reason (e.g. `GitHub App was uninstalled from or lost access to <owner/repo>`) — rather than the session breaking later on a silent 401. The webhook terminates on the controller (it serves public ingress); the in-memory eviction is broadcast to every worker via the **controller→worker eviction seam** so each worker busts its own cache too (best-effort — a dead/unreachable worker is logged, not fatal; it self-corrects at its next mint). *Note:* the controller→worker channel is not yet wired (deferred to #134/#151), so today the broadcast is a no-op and the eviction is controller-local — each worker still self-corrects on its own TTL lapse / next-mint `InstallationGone`. (That same `InstallationGone` backstop also covers the case where no webhook is configured at all.)

**REST rate-limit note (N-worker cold start).** Because resolution is stateless and per-pod, each worker that has never resolved a given repo issues **one** `GET …/installation` probe on its cold cache. A fleet of N workers can therefore issue up to **N probes per distinct repo per TTL window**. These draw from the shared 5000/hr GitHub REST budget alongside token minting, journaling, goals CRUD, and authz. The jittered TTL spreads the re-probe over a ±5-minute window to avoid a synchronized stampede; sizing the fleet and the TTL keeps the aggregate probe rate well under budget.

**New-repo install bridge (personal + org).** After `create_repo` succeeds on a goal trigger, the App is **probed** on the new repo before session work begins. A repo created on the user's behalf does **not** guarantee the App is installed (installation is a separate, interactive consent — fkst-hosted never auto-installs). When the App is not installed, the trigger returns `422` with the install URL as actionable guidance. **Org-aware:** because the App requests the `administration` permission (#110), an org install is **owner-gated** — repo admins are excluded and only an org **owner** can install/approve it (a non-owner can only *request* it). The hint copy for an org repo says "ask an organization OWNER to install/approve" and never implies self-install. An installation that exists but whose requested permission is **still pending the owner's approval** is treated as a distinct *awaiting-owner-approval* state (a `422` with that wording), not "not installed".

### How `git` actually uses the token (credential helper + rotation, #107)

fkst-substrate is **reference-only**: it runs `git`/`gh`/`codex` as child processes and we cannot change how they authenticate. **All of the wiring below is configured from fkst-hosted's side** — the engine-process environment and the runtime dir it owns — with **zero changes to fkst-substrate's `crates/`**. The lever is that substrate builds its child processes **without** `env_clear`, so they inherit the environment and runtime dir fkst-hosted lays down.

Plain `git push` over HTTPS does **not** read `GITHUB_TOKEN`; it consults a credential helper. So setting `GITHUB_TOKEN` alone (as #106 does) authenticates `gh` but not `git`. The credential delivery has three cooperating pieces:

1. **A rotatable token file** — `<runtime_dir>/github-token`, mode `0600`, holding JSON `{ "token": "ghs_…", "expires_at": "<RFC3339>" }`. (Pre-#107 this was a bare token string with no freshness signal.) It is written **atomically** (sibling tmp file + `rename`, atomic on the same filesystem) by **both** the startup write and the periodic/reactive/JIT refresh — through one shared writer (`engine::write_token_file`), so the on-disk format and the atomicity guarantee never diverge. A concurrent reader therefore never sees a torn file.

2. **A credential helper script** — `<runtime_dir>/git-credential-fkst`, mode `0700`, a small `/bin/sh` script materialized into the workspace. As a git credential helper it reads the token file and prints `username=x-access-token` + `password=<token>`. It holds **no** private key and **cannot** mint. Near expiry (the on-disk `expires_at` is within a safety window, default 5 min) it performs a **just-in-time re-mint**: it drops a nonce-bearing request file (`<runtime_dir>/github-token.request`) that the driver services (mint → atomic rewrite of the token file → delete the request file), waits bounded on that deletion, then re-reads. If no fresh token arrives in the window it **falls back to the current token** rather than failing `git` hard — the periodic backstop still covers true expiry. JSON is parsed with anchored POSIX `grep`/`sed` (no `jq`/`python3` dependency).

3. **Git config injected via env** — on the engine process fkst-hosted sets `GIT_CONFIG_COUNT` + `GIT_CONFIG_KEY_i`/`GIT_CONFIG_VALUE_i` pairs for:
   - `credential.https://github.com.helper = !<absolute helper path>` (git's shell-exec helper form),
   - `credential.https://github.com.useHttpPath = false` (one credential serves every path under the host),
   - `url.https://github.com/.insteadOf = git@github.com:` (coerce scp-style SSH remotes to HTTPS so the helper applies).

   The token is **never** embedded in a remote URL or written to any on-disk `.git/config` (leak surface). These keys, plus `GH_TOKEN` and `FKST_GITHUB_MINT_NONCE`, are **platform-reserved** (`is_reserved_env_key`), so a user-supplied `env_profile` can never shadow them and redirect the helper.

**On-demand mint authentication.** The JIT mint request is authenticated to the session by a per-session 128-bit **nonce**: written once at startup to `<runtime_dir>/.mint-nonce` (mode `0600`) and handed to the helper via the reserved `FKST_GITHUB_MINT_NONCE` env var. The driver's poller mints only when the request file's nonce matches the nonce file, so only that session's own engine child can trigger its own re-mint. No new network listener or socket is introduced; the protocol is entirely within the `0600` runtime dir.

**Hardened rotation backstop.** The periodic refresh cooldown **tightens** as expiry approaches (a flaky mint near the deadline is retried every 10 s instead of every 60 s). On a persistent mint failure **with the on-disk token already expired**, or on an `InstallationGone` (the App was uninstalled mid-session), the driver stops the engine and transitions the session to `Failed` with a clear reason — rather than letting substrate keep hitting a silent 401 with a dead token. The token value and the mint nonce are `SecretString` and are never logged.

**The `gh` exception.** `GITHUB_TOKEN`/`GH_TOKEN` are still set on the engine process for `gh`'s convenience, but they are **frozen at spawn** and a GitHub App installation token is **~1 h and non-renewable** — so `gh` cannot rotate past the TTL. Treat substrate's `gh` as best-effort short reads early in the session; durable issue/PR work should route through fkst-hosted's own `github_hub` path. `git`, by contrast, rotates for the whole session via the helper + file above.

### Repository creation requires `repo` scope (NyxID-side config)

User-attributed repository creation (`create_repo`, reachable via `POST /api/v1/goals/{id}/trigger` with `repo_mode=create_new`) runs on the **NyxID credential-injection proxy path** — fkst-hosted never sees the user's GitHub token. Creating a repository requires the GitHub **`repo`** OAuth scope (or `public_repo` for public-only). The seeded NyxID `github` provider requests only `read:user` and `user:email` by default, so a connection made with those defaults will be rejected by GitHub with a 403.

**The scope grant is NyxID/deployment configuration, not fkst-hosted code.** There are two ways to obtain a repo-capable connection:

- **Expand the `github` provider's `default_scopes`** to include `repo` (NyxID admin `PUT /api/v1/providers/{id}`, or pass `additional_scopes` at connect time) and have the user (re-)connect GitHub granting repo access, **or**
- **Use the seeded `github-pat` provider** with a `repo`-scoped Personal Access Token.

When the linked connection lacks the scope, fkst-hosted does not fail opaquely: it returns **422 Unprocessable** with an actionable hint telling the user to reconnect GitHub with repo access (rather than a generic auth failure).

**Org-repo creation** (`POST /orgs/{org}/repos`) has extra user-token failure modes, each surfaced as an actionable 422:

- **SAML SSO not authorized** — if the org enforces SAML SSO and the proxied OAuth token is not SSO-authorized for it, GitHub returns a 403 with an `X-GitHub-SSO` header carrying a (~1h) authorization URL. fkst-hosted forwards that URL in the error so the user can authorize their token for the org. (GitHub App *installation* tokens are auto-SSO-authorized, so this affects only the user-token repo-creation path, not substrate session work.)
- **OAuth app not org-approved / org policy** — the org may forbid members from creating repos, require app approval, or restrict visibility (a non-owner requesting `private` gets a 422). These are surfaced as an org-policy error distinct from the missing-scope case.

> Re-consent / SSO-authorize UI is frontend + NyxID scope; fkst-hosted only surfaces the backend error semantics and the authorization URL.

---

## Operations Runbook

### 1. Create the GitHub App

1. Go to **GitHub Settings > Developer settings > GitHub Apps > New GitHub App** (under the ChronoAI org).
2. Fill in:
   - **GitHub App name:** `fkst-hosted` (or your preferred name).
   - **Homepage URL:** `https://github.com/ChronoAIProject/fkst-hosted`.
   - **Webhook:** uncheck **Active** (webhooks are OFF in v1).
   - **Repository permissions (session admin set — issue #110):**
     - **Administration:** Read & Write (admin-equivalent: branch protection / rulesets, collaborators & teams, repo settings, visibility, rename/transfer, deploy keys).
     - **Pull requests:** Read & Write (open / update / merge PRs).
     - **Contents:** Read & Write (clone, push progress records, read/write session files).
     - **Issues:** Read & Write (create / comment / close issues).
     - **Metadata:** Read (required by GitHub; always read; granted implicitly on installation tokens).
   - All four of **Administration, Pull requests, Contents, Issues** are a **hard prerequisite** — the token mint can only request a subset of what the App was granted, so a mint **422s** (`github token request rejected`) for any one of them the App lacks.
   - `workflows`, `secrets`, `actions`, `repository_hooks`, and `environments` are **deliberately NOT requested** (the session token cannot touch `.github/workflows/**` or manage Actions secrets — file a follow-up if needed).
   - **Where can this GitHub App be installed?** Any account (or restrict to ChronoAI org if preferred). Note: because the App requests **Administration**, **organization** installs are **owner-only** — repo admins cannot install it; non-owner members can only *Request* it for an org owner to approve.
3. Click **Create GitHub App**.
4. Note the **App ID** (visible on the app's settings page).

### 2. Generate and Store the Private Key

1. On the app's settings page, scroll to **Private keys**.
2. Click **Generate a private key**. A `.pem` file downloads.
3. Store the key securely:
   - **Option A (recommended):** Set `FKST_GITHUB_APP_PRIVATE_KEY_PEM` in the Kubernetes Secret (use literal `\n` escapes for newlines in the single-line stringData value):
     ```yaml
     stringData:
       FKST_GITHUB_APP_PRIVATE_KEY_PEM: "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----"
     ```
   - **Option B:** Mount the `.pem` file as a volume and set `FKST_GITHUB_APP_PRIVATE_KEY_PATH` in the ConfigMap:
     ```yaml
     data:
       FKST_GITHUB_APP_PRIVATE_KEY_PATH: "/etc/fkst-github-app/key.pem"
     ```
4. Set `FKST_GITHUB_APP_ID` in the ConfigMap or Secret to the App ID noted above.
5. Optionally set `FKST_GITHUB_APP_SLUG` to the URL-friendly slug shown on the app's settings page (used in install-hint URLs in error messages).

### 3. Install the App on Target Repos

1. Go to the app's settings page > **Install App**.
2. Click **Install** next to the target org/user.
   - **Organization targets are owner-only:** because the App requests the **Administration** permission, only an **org owner** can install (or approve) it. Repo admins are excluded; a non-owner member can only **Request** the install, which an org owner then approves.
3. Select the repositories the App should access (or "All repositories").
4. Click **Install**.

The App is now ready. fkst-hosted will detect the configuration at next startup and log `github app enabled (app_id=...)`.

> **Re-consent on existing installations (issue #110):** adding the `administration` permission to an already-published App **suspends that permission on every existing installation until the owner re-approves**. Roll it out by declaring the admin set in the App settings **before** broad installation so new installs consent up-front; existing installations receive a review prompt and must be re-approved (an **org owner** for org installs) before the elevated scope takes effect.

### 4. Key Rotation

GitHub allows two active private keys simultaneously. Use this for zero-downtime rotation:

1. Generate a new private key on the app's settings page (the old key remains active).
2. Update the Kubernetes Secret or mounted file with the new key.
3. Roll the deployment (`kubectl rollout restart deployment/fkst-hosted-api`).
4. Verify the new pods start successfully and log `github app enabled`.
5. Delete the old private key from GitHub.

### 5. Troubleshooting

| Symptom | Likely Cause | Resolution |
|---------|-------------|------------|
| Startup fails with "PEM does not parse as a valid RSA private key" | Corrupted or wrong-format PEM | Verify the PEM is a valid RSA private key (PKCS#1 or PKCS#8). Check that `\n` escapes are present if using `_PEM` env var. |
| Startup fails with "both FKST_GITHUB_APP_PRIVATE_KEY_PEM and FKST_GITHUB_APP_PRIVATE_KEY_PATH set" | Conflicting config | Provide exactly one key source. Remove the other. |
| Startup fails with "FKST_GITHUB_APP_ID set without FKST_GITHUB_APP_PRIVATE_KEY_PEM or _PATH" | Missing key | Provide a private key via one of the two env vars. |
| 422 `github app not installed on owner/repo` with install URL | App not installed on that repo | Install the App on the target repo (see step 3). The error message includes the install URL when the slug is configured. |
| 422 `github app not installed on owner/repo` without install URL | App slug not configured | Set `FKST_GITHUB_APP_SLUG` to get actionable install URLs in error messages. |
| 503 `github rate limited` | API rate limit exhausted | Wait for the reset period (indicated in the error). Per-installation rate limits apply. |
| 422 `github token request rejected` | Permission subset exceeds App's granted permissions (e.g. the App lacks `administration`), or an existing install has not re-approved the new admin scope | Verify the App declares **Administration, Pull requests, Contents, Issues** all at **Read & write** (issue #110) and that the installation has re-approved (org installs need an **org owner**). The rejected-permission detail is logged at the mint site (`github installation-token mint rejected (422)`). |
| 422 "missing the `repo` permission needed to create repositories" | Linked GitHub connection lacks the `repo` OAuth scope | Reconnect GitHub granting repo access, expand the NyxID `github` provider's `default_scopes` to include `repo`, or use the `github-pat` provider with a `repo`-scoped token (see "Repository creation requires `repo` scope" above). |
| 422 "not SSO-authorized for the `<org>` organization" | Org enforces SAML SSO; the proxied token is not authorized for it | Authorize your GitHub token for the org via the URL surfaced in the error (expires ~1h), then retry. |
| 422 "organization's policy prevents creating this repository" | Org forbids member repo creation, the OAuth app is not org-approved, or the requested visibility is disallowed | Have an org owner allow repo creation / approve the OAuth app, or request a permitted visibility. |
| Module disabled at startup (`github app disabled`) | `FKST_GITHUB_APP_ID` unset | This is normal if the App is not yet configured. Set the env var to enable. |

### 6. Environment Variables Reference

| Variable | Required | Description |
|----------|----------|-------------|
| `FKST_GITHUB_APP_ID` | No (unset = disabled) | The GitHub App's numeric ID. |
| `FKST_GITHUB_APP_PRIVATE_KEY_PEM` | Yes (if ID set) | Inline PEM content. Literal `\n` escapes are normalized to real newlines. Mutually exclusive with `_PATH`. |
| `FKST_GITHUB_APP_PRIVATE_KEY_PATH` | Yes (if ID set) | File path to a PEM file. Mutually exclusive with `_PEM`. |
| `FKST_GITHUB_APP_SLUG` | No | URL-friendly slug for install-hint URLs (e.g. `fkst-hosted`). |
| `FKST_GITHUB_APP_WEBHOOK_SECRET` | No (recommended) | HMAC secret for `POST /api/v1/github/app/webhook` (issue #108). When set the webhook route is mounted and signature-verified, giving a **prompt cache-bust** on uninstall/repo-removal; when unset the route is not mounted (a warning is logged) and staleness is corrected lazily at the next mint via the `InstallationGone` backstop (resolution is on-demand + in-memory cached either way, #141). Held in a `SecretString`; never logged. |
