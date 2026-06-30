# fkst-hosted

**fkst-hosted** is ChronoAI's hosted cloud service for the **fkst** project. It
gives you a managed home for your fkst packages and the engine that runs them —
so you can build, run, and collaborate without operating any infrastructure
yourself.

## What you can do

- **Keep your fkst packages in one place.** Create, update, and organize your
  packages (the lua bundles the engine runs), or upload them as a zip.
- **Generate a package from a description.** Describe what you want in plain
  language and get a ready-to-run package draft back.
- **Share with your team.** Give other people — or a whole organization —
  permission to view or run a package.
- **Run your packages.** Start an engine session, follow it while it runs, and
  stop it whenever you like.
- **Pursue goals against GitHub.** Capture a goal — an intent plus the packages
  to use — point it at a GitHub repository (existing, or created for you), and
  trigger it when you're ready.
- **Manage GitHub issues from one place.** See the issues across all of your
  linked GitHub accounts, and create, update, or comment on them.

Everything is reached through a simple HTTP API and secured by your ChronoAI
(NyxID) sign-in.

## Using the API

The **[HTTP API Reference](docs/api-reference.md)** documents everything you need
to build against fkst-hosted: every endpoint, how to authenticate, the
permissions each call requires, request/response formats, and worked examples.

## Local Development & Auth

For instructions on setting up authentication, service accounts, and GitHub integrations using local vs production profiles, see the **[Authentication & GitHub Integration Guide](docs/auth-integration.md)**.

---

<sub>Running or contributing to fkst-hosted? See
[`backend/README.md`](backend/README.md) for local development and the
per-deployable Kubernetes samples
[`backend/k8s_sample/README.md`](backend/k8s_sample/README.md)
and
[`backend/fkst-worker/k8s_sample/README.md`](backend/fkst-worker/k8s_sample/README.md)
for deployment.</sub>
