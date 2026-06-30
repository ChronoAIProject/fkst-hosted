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

The control plane serves a live **OpenAPI 3.1** document at `GET /openapi.json`,
generated at runtime from the actual routes — every public endpoint, its
authentication, the permissions it requires, and its request/response shapes.

---

<sub>Deploying fkst-hosted? See the Kubernetes samples
[`backend/k8s_sample/README.md`](backend/k8s_sample/README.md).</sub>
