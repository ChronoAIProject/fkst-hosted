# fkst-hosted session API reference

Use this reference for exact session creation calls. Fetch `GET /openapi.json` from the
target deployment when its live contract must take precedence over this checked-in copy.

## Authentication and authorization

Send `Authorization: Bearer <github-token>` and JSON over HTTPS. The control plane verifies
the token's GitHub identity and deployment allowlist. Creating a session requires the
caller's repository role to be `admin` or `maintain`, unless the caller is a verified
global admin.

## Create a session

`POST /api/v1/repos/{owner}/{name}/sessions` with `Content-Type: application/json` creates
an `fkst-substrate-trigger` issue as the authenticated user.

| JSON field | Required? | Meaning |
|---|---|---|
| `name` | yes | session name and issue title; same validation as `### Session Name` |
| `packages` | yes | package-ref array; may be empty only when `manifests` is non-empty |
| `manifests` | no | manifest-ref array; defaults to `[]` |
| `work_label` | no | one explicit work label; a collision returns `409` |
| `environment` | no | reusable profile name; exclusive with `disposable_environment` |
| `disposable_environment` | no | request-only object described below |
| `source_branch` | no | source branch; repository default when omitted |
| `target_branch` | no | target branch; `fkst-hosted-default` when omitted |
| `auto_merge` | no | only JSON `true` enables it |
| `log_access` | no | array of individual GitHub logins/ids granted log access and trust |
| `collaborators` | no | array of individual GitHub logins/ids granted work-item authority |
| `output_lang` | no | session output locale |

Do not combine multiple logins in one `log_access` or `collaborators` entry. The renderer
proves that every field round-trips through the GitHub trigger parser before writing the
issue. The API does not accept `Engine Config`.

### Disposable environment object

`disposable_environment` accepts three default-empty fields:

| Field | Type | Rule |
|---|---|---|
| `install` | string array | ordered shell commands; at most the configured command cap |
| `variables` | string map | non-secret process environment variables |
| `secrets` | string map | write-only secret process environment variables |

At least one field must contain an entry. `environment` and `disposable_environment` are
mutually exclusive. Variable and secret keys must match `^[A-Za-z_][A-Za-z0-9_]*$`, cannot
be platform-reserved, and cannot overlap. Deployment defaults are 50 commands, 4,096 bytes
per command, 100 entries per variable/secret map, and 65,536 bytes per value.

OpenAPI marks secret property names and values `writeOnly`. The `201` schema never includes
the disposable object. A `422` validation message may identify an offending key or command
index/length, but never returns submitted values or command text.

Example with placeholders only:

```json
{
  "name": "release-session",
  "packages": ["owner/repo@main:packages/release"],
  "disposable_environment": {
    "install": ["apt-get update && apt-get install -y jq"],
    "variables": {"APP_MODE": "release"},
    "secrets": {"DEPLOY_TOKEN": "<secret-value>"}
  }
}
```

Keep the request body out of client, proxy, and shell-history logs. The GitHub issue stores
only this fixed `### Environment` value:

```
Disposable one-time environment. Details are injected privately into the session sandbox and are not stored in this GitHub issue.
```

The creator-bound payload remains only in the receiving process until the sandbox backend
accepts it. Success consumes it; a failed sandbox creation retains it for retry; closing
the trigger removes it. Restart/failover before handoff or any later runtime recreation
cannot recover it and fails closed. Close the trigger and create a new session; never put
the missing private details in GitHub.

## Responses

Success is `201` with
`{"issue_number": 123, "html_url": "https://github.com/owner/repo/issues/123"}`. Errors use
`{"error": "<stable_code>", "message": "<client-safe text>"}`.

| Status | Meaning |
|---|---|
| `400` | malformed/inconsistent fields, parser round-trip failure, or GitHub rejected the issue |
| `401` | missing or invalid GitHub token |
| `403` | deployment access denied, insufficient repository role, or GitHub refused the write |
| `404` | repository absent or invisible to the caller |
| `409` | explicit work label collides with another open session |
| `422` | invalid disposable contents/branches, disabled issues, or another semantic precondition |
| `503` | GitHub unavailable or this process cannot provide the disposable handoff |

There is no disposable read/update endpoint. Replace incorrect details by calling
`DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}`, then create a new session.
