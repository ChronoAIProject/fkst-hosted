# fkst-hosted

**fkst-hosted** turns GitHub issues into autonomous coding sessions. Install the
GitHub App on a repository, describe work as issues, and receive a pull request
for each task without operating the session infrastructure yourself.

## Current capabilities

- **Run coding sessions for your repositories.** Declare each session with a
  GitHub trigger issue and configure the workflows and environment it should use.
- **Queue work with issues.** Add focused work items, follow their status, and
  review the pull requests the session creates.
- **Work from GitHub or the dashboard.** Use issues as the durable source of
  truth, or sign in with GitHub for a visual view of repositories and sessions.
- **Inspect and control sessions.** Start or stop sessions, manage environments
  and GitHub App installations, and review live state, logs, and outcomes.
- **Review your own activity and sandboxes.** The **Operations** view shows the
  API calls you made and the live sandboxes you own or were explicitly given
  access to. It is scoped to you: sharing a session never exposes another
  person's API activity, and a deployment administrator is the only role that
  can see across users.
- **Automate through REST.** Use the dashboard's machine-readable API for
  supported session, work-item, environment, log, and outcome operations.

## Get started

1. Install the fkst-hosted GitHub App on the repositories where sessions should
   run.
2. Start a session from the dashboard or the installed **fkst substrate
   session** issue template.
3. Queue a task from the dashboard or the **fkst work item** issue template,
   then follow its issue status and review the resulting pull request.

See the [fkst-hosted user manual](skills/fkst-control-plane-manual/SKILL.md) for
session configuration, work labels, environments, permissions, and lifecycle
details.

## Repository layout

- `backend/` - Rust control plane, GitHub reconciliation, runtime dispatch, and
  HTTP API
- `frontend/` - React web application, user guide, and authenticated dashboard
- `deploy/kubernetes/` - Kubernetes manifests, validation tools, and recovery
  runbooks
- `skills/fkst-control-plane-manual/` - canonical user and operator contract
- `apps/local-qa-runtime/` - independently buildable Local QA Host and reserved
  hardened Runtime shells
- `packages/qa-contracts/` - shared Local QA contracts and Rust/TypeScript fixtures

Local QA Host starts through the explicit loopback-only
`local-demo --listen <loopback> --database <path>` command. It persists Runs,
ordered Events, and cancellation intent in a migrated SQLite WAL journal.
Production v2 admission remains fail-closed without a current-claim authority
adapter; the real Worker/Browser walk is gated behind `mvp0-browser-test` and
is not production composition. The launcher, supervisor, guest agent, and Secret
Broker remain inert shells. See
[`apps/local-qa-runtime/README.md`](apps/local-qa-runtime/README.md) for the
supported HTTP routes, verification commands, and deferred capabilities.

Kernel-engine code remains upstream in `fkst-substrate`, and upstream engine
and package repositories are reference-only from this checkout. The FKST Cloud
package catalog resides on this repository's `packages` branch.

## Development

Run the frontend development server (it proxies `/api` to the control plane on
port `8080`):

```bash
cd frontend
npm ci
npm run dev
```

Use `npm run typecheck`, `npm run lint`, `npm run test`, and `npm run build` for
the frontend's local verification gates.

## API and deployment

The control plane serves its runtime-generated **OpenAPI 3.1** contract at
`GET /openapi.json`. Use that contract as the authority for available routes,
request and response shapes, and each operation's authentication requirements.

For self-hosting, follow the
[FKST Local Deployment Guide](CLAUDE.md#fkst-local-deployment-guide). Checked-in
namespace deployment sources and validation commands are documented in
[`deploy/kubernetes/README.md`](deploy/kubernetes/README.md).

The activity trace behind the Operations view is optional and operator-owned:
[AUDIT-TRACE.md](deploy/kubernetes/AUDIT-TRACE.md) documents its architecture,
data boundaries, authorization model, and retention;
[AUDIT-RUNBOOK.md](deploy/kubernetes/AUDIT-RUNBOOK.md) documents provisioning,
rollout, and incident response.
