# fkst-hosted

**fkst-hosted** turns GitHub issues into autonomous coding sessions. Install the
GitHub App on a repository, describe work as issues, and receive a pull request
for each task without operating the session infrastructure yourself.

## What you can do

- **Run coding sessions for your repositories.** Declare each session with a
  GitHub trigger issue and configure the workflows and environment it should use.
- **Queue work with issues.** Add focused work items, follow their status, and
  review the pull requests the session creates.
- **Work from GitHub or the dashboard.** Use issues as the durable source of
  truth, or sign in with GitHub for a visual view of repositories and sessions.
- **Inspect and control sessions.** Start or stop sessions, manage environments
  and GitHub App installations, and review live state, logs, and outcomes.
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

## API and deployment

The control plane serves its runtime-generated **OpenAPI 3.1** contract at
`GET /openapi.json`. Use that contract as the authority for available routes,
request and response shapes, and each operation's authentication requirements.

For self-hosting, follow the
[FKST Local Deployment Guide](CLAUDE.md#fkst-local-deployment-guide).
