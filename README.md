# fkst-hosted

**fkst-hosted** contains ChronoAI's user-facing products for the **fkst**
project. Its hosted control plane gives you a managed home for fkst packages and
the engine sessions that run them, while the Local QA Runtime provides the
repository boundary for local QA tooling.

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

## Repository boundaries

- `backend/`, `frontend/`, and `deploy/` own the hosted control plane and web
  experience.
- `apps/local-qa-runtime/` owns the user-facing Local QA Runtime. It is currently
  an inert, independently buildable scaffold; runtime protocols and behavior
  belong to later, separately reviewed changes.
- Kernel-engine code remains upstream in `fkst-substrate`. Shared fkst packages
  remain upstream in `fkst-packages`; both repositories are reference-only from
  this checkout.

The hosted capabilities are reached through a simple HTTP API and secured by
your ChronoAI (NyxID) sign-in.

## Using the API

The control plane serves a live **OpenAPI 3.1** document at `GET /openapi.json`,
generated at runtime from the actual routes — every public endpoint, its
authentication, the permissions it requires, and its request/response shapes.

---

<sub>Deploying fkst-hosted? See the **FKST Local Deployment Guide** section in
[`CLAUDE.md`](CLAUDE.md#fkst-local-deployment-guide).</sub>
