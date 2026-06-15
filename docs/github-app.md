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
- Adding the `administration` permission to the App would trigger per-installation re-approval; it is deliberately excluded from v1.

### Session-token delivery to the engine

A goal session's substrate engine receives its installation token **at t=0**, before the engine process is started (issue #106). The session driver mints the repo-scoped installation token and builds a `GoalContext`, then starts the engine via `start_with_spec(goal: Some(..))`. As a result, before the engine runs the driver has:

- written `<runtime_dir>/github-token` (mode `0600`) and `<runtime_dir>/goal.json`, and
- set `GITHUB_TOKEN`, `FKST_GITHUB_TOKEN_FILE`, and `FKST_GOAL_FILE` on the substrate child process.

The same `GoalContext` path is used identically by the initial start and by the failover rebuild on a takeover pod — the token is never persisted, always (re-)minted from the `SessionDoc`. Minting is a cache hit after the trigger-time install-check preflight (`token_for_repo` caches per `(repo, perms)`), so this is not a second network mint. The in-run periodic refresh then re-mints ~55 minutes later (5 minutes before the 60-minute TTL).

> Earlier (pre-#106) the driver started the engine with `goal: None` and the token only reached the runtime dir via the periodic refresh, which was suppressed for the first ~55 minutes — so the engine ran with no credential at startup. That regression is fixed: the token is present from t=0.

---

## Operations Runbook

### 1. Create the GitHub App

1. Go to **GitHub Settings > Developer settings > GitHub Apps > New GitHub App** (under the ChronoAI org).
2. Fill in:
   - **GitHub App name:** `fkst-hosted` (or your preferred name).
   - **Homepage URL:** `https://github.com/ChronoAIProject/fkst-hosted`.
   - **Webhook:** uncheck **Active** (webhooks are OFF in v1).
   - **Repository permissions (v1):**
     - **Contents:** Read & Write (clone, push progress records, read/write session files).
     - **Metadata:** Read (required by GitHub; always read).
     - **Issues:** Read & Write (dormant in v1; included so the journaling issue-comment mirror can later ride installation tokens).
     - **Administration:** No access (deliberately excluded; org-repo creation is a later feature and adding permissions triggers per-installation re-approval).
   - **Where can this GitHub App be installed?** Any account (or restrict to ChronoAI org if preferred).
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
3. Select the repositories the App should access (or "All repositories").
4. Click **Install**.

The App is now ready. fkst-hosted will detect the configuration at next startup and log `github app enabled (app_id=...)`.

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
| 422 `github token request rejected` | Permission subset exceeds App's granted permissions | Verify the v1 permissions (Contents R&W, Metadata R, Issues R&W) are granted on the installation. |
| Module disabled at startup (`github app disabled`) | `FKST_GITHUB_APP_ID` unset | This is normal if the App is not yet configured. Set the env var to enable. |

### 6. Environment Variables Reference

| Variable | Required | Description |
|----------|----------|-------------|
| `FKST_GITHUB_APP_ID` | No (unset = disabled) | The GitHub App's numeric ID. |
| `FKST_GITHUB_APP_PRIVATE_KEY_PEM` | Yes (if ID set) | Inline PEM content. Literal `\n` escapes are normalized to real newlines. Mutually exclusive with `_PATH`. |
| `FKST_GITHUB_APP_PRIVATE_KEY_PATH` | Yes (if ID set) | File path to a PEM file. Mutually exclusive with `_PEM`. |
| `FKST_GITHUB_APP_SLUG` | No | URL-friendly slug for install-hint URLs (e.g. `fkst-hosted`). |
