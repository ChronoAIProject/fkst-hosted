# Authentication & GitHub Integration Guide

This document describes the environment-driven authentication posture and GitHub App/Proxy integration for `fkst-hosted`. It aligns with the endpoints and behaviors documented in [docs/api-reference.md](file:///Users/chronoai/Desktop/aelf-frontend-work/FKST/_wt/dev-5/docs/api-reference.md) and the operational rules in [docs/github-app.md](file:///Users/chronoai/Desktop/aelf-frontend-work/FKST/_wt/dev-5/docs/github-app.md).

---

## 1. Environment Variables Matrix

The following matrix covers all environment variables supported by the backend and frontend configurations.

### Backend Configurations (defined in `backend/fkst-hosted-api/src/config.rs`)

| Variable | Scope / Required? | Default | Profile (A) DEFAULT/Prod | Profile (B) DEV/Observer | Role & Description |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`MONGODB_URI`** | **Yes** | — | `mongodb://mongodb-prod:27017` | `mongodb://localhost:27017` | MongoDB connection string. Redacted in logs. |
| **`MONGODB_DB`** | No | `fkst_hosted` | `fkst_hosted` | `fkst_hosted_dev` | Logical MongoDB database name. |
| **`MONGODB_SERVER_SELECTION_TIMEOUT_MS`** | No | `5000` | `5000` | `5000` | Timeout (ms) for MongoDB connection and health ping. |
| **`FKST_HOSTED_PORT`** | No | `8080` | `8080` | `8080` | TCP port the API server binds to. |
| **`FKST_HOSTED_BIND_ADDR`** | No | `0.0.0.0` | `0.0.0.0` | `0.0.0.0` | Bind IP address for the HTTP server. |
| **`FKST_HOSTED_LOG_LEVEL`** | No | `info` | `info` | `debug` | Log level for the backend API. |
| **`FKST_HOSTED_REQUEST_TIMEOUT_SECS`** | No | `30` | `30` | `30` | Request timeout limits in seconds. |
| **`FKST_AUTH_ENABLED`** | No | `true` | `true` | `false` | Master JWT validation switch. If `false`, routes are open. |
| **`FKST_AUTH_NYXID_BASE_URL`** | When auth on | — | `https://nyx.chrono-ai.fun` | — | Base URL of NyxID IAM for JWKS keys extraction. |
| **`FKST_AUTH_ISSUER`** | No | `nyxid` | `nyxid` | — | Expected JWT `iss` claim in the incoming token. |
| **`FKST_AUTH_AUDIENCE`** | No | base URL | `https://nyx.chrono-ai.fun` | — | Expected JWT `aud` claim (defaults to base URL). |
| **`FKST_AUTH_JWKS_CACHE_TTL_SECS`** | No | `300` | `300` | — | JWKS key cache TTL in seconds. |
| **`NYXID_CLIENT_ID`** | No (Both-or-none)| — | `sa_REPLACE_ME_prod` | — | NyxID service-account client ID for org membership check. |
| **`NYXID_CLIENT_SECRET`** | No (Both-or-none)| — | `sa_sec_REPLACE_ME_prod` | — | NyxID service-account client secret. |
| **`FKST_NYXID_ORG_CACHE_TTL_SECS`** | No | `30` | `30` | — | Cache TTL for NyxID org membership queries. |
| **`FKST_GITHUB_APP_ID`** | No (app enabled) | — | `123456` | — | Numeric ID of the fkst-hosted GitHub App. |
| **`FKST_GITHUB_APP_PRIVATE_KEY_PEM`** | If ID set (no PATH)| — | `"-----BEGIN RSA..."` | — | Inline PEM key content with `\n` normalization. |
| **`FKST_GITHUB_APP_PRIVATE_KEY_PATH`** | If ID set (no PEM) | — | — | — | Path to the private key PEM file. |
| **`FKST_GITHUB_APP_SLUG`** | No | — | `fkst-hosted` | — | Slug of the app (used for install-hint URLs in 422s). |
| **`FKST_GITHUB_WRITE`** | No | `DRY-RUN` | `REAL` | `DRY-RUN` | Engine/deployment-level — not consumed by fkst-hosted-api; governs the substrate dev-loop write posture. |
| **`FKST_HOSTED_LLM_GATEWAY_URL`** | No | — | `https://nyx.chrono-ai.fun/...` | — | NyxID LLM-gateway base URL for package generation. |
| **`FKST_HOSTED_LLM_MODEL`** | When gateway set| — | `claude-3-5-sonnet` | — | Model name required by the LLM gateway. |
| **`FKST_HOSTED_LLM_TIMEOUT_SECS`** | No | `20` | `20` | — | LLM completion request timeout limit. |
| **`FKST_HOSTED_LLM_MAX_OUTPUT_BYTES`** | No | `1048576` | `1048576` | — | Limit on LLM generation output size. |
| **`GITHUB_TOKEN`** | No | — | `ghp_REPLACE_ME` | — | Deploy-level token for backing up journaling repositories. |
| **`FKST_JOURNAL_GITHUB_ENABLED`** | No | `true` | `true` | `false` | Enable/disable git journaling. |
| **`FKST_JOURNAL_GITHUB_REPO`** | No | — | `ChronoAI/journals` | — | Dedicated repository where session logs are pushed. |
| **`FKST_JOURNAL_GITHUB_BRANCH`** | No | `main` | `main` | — | Branch name for journal logs. |
| **`FKST_JOURNAL_FLUSH_INTERVAL_MS`** | No | `2000` | `2000` | — | Debounce buffering delay (ms) before pushing commits. |
| **`FKST_JOURNAL_FLUSH_MAX_BATCH`** | No | `50` | `50` | — | Flush buffer early if this count is reached. |
| **`FKST_JOURNAL_ISSUE_COMMENTS`** | No | `false` | `false` | — | Mirror logs as issue comments on GitHub. |
| **`FKST_JOURNAL_CAS_MAX_RETRIES`** | No | `5` | `5` | — | Retries allowed for optimistic concurrency failures. |
| **`FKST_RAISED_IDENTITY_POINTERS`** | No | `/department...` | `/department...` | `/department...` | JSON pointers identifying unique raised events. |
| **`FKST_RAISED_MAX_LINE_BYTES`** | No | `1048576` | `1048576` | `1048576` | Max stdout line length parsed. |

### Frontend Configurations (defined in `frontend/.env.example`)

| Variable | Required? | Default | Profile (A) DEFAULT/Prod | Profile (B) DEV/Observer | Role & Description |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`VITE_FKST_API_BASE`** | No | Same origin | `https://api.hosted.chronoai.co` | `http://127.0.0.1:8080` | Backend endpoint for the fkst-hosted API. |
| **`VITE_NYXID_BASE_URL`** | When auth on | — | `https://nyx.chrono-ai.fun` | — | Base URL of NyxID IAM for redirects and OAuth flow. |
| **`VITE_NYXID_CLIENT_ID`** | When auth on | — | `fkst-hosted-frontend` | — | Registered OAuth client ID with NyxID. |
| **`VITE_NYXID_REDIRECT_URI`** | When auth on | — | `https://hosted.chronoai.co/auth/callback` | — | OAuth callback URL for redirecting authentication codes. |
| **`VITE_NYXID_CONNECT_GITHUB_URL`** | When auth on | — | `https://nyx.chrono-ai.fun/api/v1/github/connect` | — | URL to trigger GitHub linkage on the NyxID credential proxy. |
| **`VITE_AUTH_REQUIRED`** | No | `true` | `true` | `false` | Master frontend auth requirement toggle. |

---

## 2. Configuration Profiles

### Profile (A): DEFAULT / Production
*   **Aesthetic Goal:** Lock-down, secure, fail-closed auth posture, automated writes/merges.
*   **Behavior:**
    *   `FKST_AUTH_ENABLED=true` forces JWT RS256 token verification at the API boundary on all endpoints except `/health`.
    *   `VITE_AUTH_REQUIRED=true` directs the frontend to enforce login screens.
    *   `FKST_GITHUB_WRITE=REAL` (engine/deployment-level — not consumed by fkst-hosted-api) tells the execution environment that it is allowed to autonomously perform writes and merge pull requests on target repositories.
    *   The GitHub App variables must be fully configured with correct credentials (PEM key or path and App ID), or the backend will fail-closed and crash at startup.

### Profile (B): DEV / Observer
*   **Aesthetic Goal:** Lightweight, zero-configuration local debugging.
*   **Behavior:**
    *   `FKST_AUTH_ENABLED=false` opens all backend routes. Authentication extracts yield developer stub contexts.
    *   `VITE_AUTH_REQUIRED=false` allows the frontend client to bypass the login redirects and work anonymously.
    *   `FKST_GITHUB_WRITE=DRY-RUN` (engine/deployment-level — not consumed by fkst-hosted-api) ensures no autonomous writes or merging operations occur during testing.
    *   The GitHub App variables can remain unset. The engine logs `github app disabled (FKST_GITHUB_APP_ID not set)` at startup and degrades gracefully.

---

## 3. User Journey ↔ API Reference Alignment

The table below describes how the client and backend handle authentication and GitHub connection steps, matching the specifications in [docs/api-reference.md](file:///Users/chronoai/Desktop/aelf-frontend-work/FKST/_wt/dev-5/docs/api-reference.md) and [docs/github-app.md](file:///Users/chronoai/Desktop/aelf-frontend-work/FKST/_wt/dev-5/docs/github-app.md).

| Step | User Journey State | Action / Flow | Endpoint / Behavior | Reference Details |
| :--- | :--- | :--- | :--- | :--- |
| **1** | **Land -> Login Redirect** | User visits the site. Token missing or expired. | Redirects browser to NyxID authorization endpoint. | Reaches `VITE_NYXID_BASE_URL` with client `VITE_NYXID_CLIENT_ID` and `VITE_NYXID_REDIRECT_URI`. |
| **2** | **Callback** | NyxID redirects browser back with authorization code. | Code is exchanged for a JWT access token. | JWT access token is stored in memory/session. Future requests include header `Authorization: Bearer <token>`. |
| **3** | **Probe Accounts** | Dashboard loads. App probes if user's GitHub is linked. | Query accounts connection status. | `GET /api/v1/github/accounts` returns `200 OK` with `AccountView[]`. Returns `503` if NyxID proxy is down. |
| **4** | **Connect-GitHub** | Returned accounts list is empty. | Prompt user to connect GitHub account. | User redirected to `VITE_NYXID_CONNECT_GITHUB_URL` (`/api/v1/github/connect`) under RFC 8693 token exchange. |
| **5** | **Issues Load** | User views issues feed. | Pull issues across connected accounts. | `GET /api/v1/github/issues`. Slow/rate-limited connections report a scoped `error.kind` on the account item, keeping overall response `200 OK`. |
| **6** | **Create Goal / Issue** | User creates a goal or issue. | Post goal and associated issue. | `POST /api/v1/goals` and `POST /api/v1/github/repos/{owner}/{repo}/issues`. Requires `account` parameter if multiple connected. |
| **7** | **401 Unauthorized** | Session token expires. | Backend rejects the request. | Returns `401 Unauthorized` with `WWW-Authenticate: Bearer` header. Frontend wipes local token and redirects to Step 1. |
| **8** | **422 App Not Installed** | Spawning a session against a repository without the App. | Backend aborts session execution. | `POST /api/v1/goals/{id}/trigger` returns `422` with `github app not installed on owner/repo`. If `FKST_GITHUB_APP_SLUG` is set, includes install URL. |
| **9** | **429 Rate Limited** | Upstream API threshold reached. | Backend throttles request. | Returns `429 rate_limited` with a `Retry-After: <seconds>` header. |
