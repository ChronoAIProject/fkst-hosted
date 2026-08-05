# Product intent — fkst-hosted

**Owner-controlled (section 12.3).** Evolution reads this file and never writes
it. It may *propose* changes, but only in a separate pull request that autonomous
policy never merges (section 12.1.1 rule 2). Everything here is a human claim
about what the product is *for*; everything in `observed/` is a machine claim
about what the product currently *does*. Keeping them apart is what stops an
inference from silently becoming product strategy.

## What this product is

fkst-hosted is ChronoAI's hosted cloud service for the fkst project. It turns a
GitHub repository into a place where autonomous coding sessions can be started,
watched, and driven — **entirely through GitHub issues**. A user opens a trigger
issue to start a session and opens labelled work issues to queue work; the
session produces pull requests.

The web dashboard is the observation and control surface over that loop. It is
not a second source of truth: every action it offers is a GitHub write performed
as the signed-in user, and every state it shows is derived live from GitHub.

## Intended audiences

- **Session owner** — the person who opened a session's trigger issue. Drives
  the session: queues work, watches health, retires it.
- **Repository maintainer** — holds admin or maintain permission on a repository
  and decides which sessions may run against it.
- **Deployment administrator** — operates the FKST Cloud deployment itself;
  spans every account and repository where the GitHub App is installed.

## Value proposition

Delegating implementation work should not mean losing sight of it. fkst-hosted
keeps the whole loop legible: what is running, what it is working on, whether it
is healthy, and what it produced — with GitHub as the only durable record, so
nothing depends on this service remaining available to reconstruct the truth.

## Canonical terminology

- A **session** is the unit of autonomous work. It is owned by an *effective
  creator* and lives in its trigger issue.
- A **work item** is an issue carrying one of the session's effective work
  labels, assigned to that session's creator.
- A **trigger issue** registers a session and freezes its configuration.
- Say **session**, not "job", "run", or "agent instance".
- Say **work item**, not "task" or "ticket".
- Say **effective creator**, not "owner", when referring to session attribution —
  "owner" is overloaded with repository ownership.

## Prohibited and regulated claims

- Never claim the product **guarantees** correct code, merged pull requests, or
  any particular outcome. Sessions produce candidate work that humans review.
- Never claim isolation or security properties beyond what the deployment
  actually enforces. Sandbox isolation depends on the runtime the deployment
  runs; do not describe it as a guarantee in artifacts that do not name that
  runtime.
- Never present a capability as available when it is gated behind configuration
  the reader may not have. State the gate.
- Never imply that fkst-hosted stores user source code. It does not; GitHub does.

## Known non-goals

- fkst-hosted is **not** the kernel engine and never changes engine internals.
  That work belongs upstream in `fkst-substrate`.
- It is not a general CI system, an issue tracker, or a replacement for code
  review.
- It does not offer an API surface for driving sessions outside the GitHub
  issue protocol. The dashboard's write endpoints are conveniences over that
  same protocol, not an alternative to it.

## Demo-data constraints

Every demonstration — screenshot, video, deck — MUST use synthetic fixtures. The
E2E fixtures in `frontend/e2e/` are the sanctioned source: they mirror the real
wire shapes field for field while naming fictional accounts (`octo-dev`,
`acme-corp`) and fictional repositories. No real repository name, real GitHub
login, real issue title, or real session identifier may appear in a public
artifact.

## Documentation voice

Direct and second-person. Say what the reader does and what happens. Name the
failure and its remedy rather than assuring the reader it will not occur. Prefer
a concrete error code or label over a paraphrase.

## Presentation audiences and confidentiality

| Deck audience        | Confidentiality | Notes                                          |
| -------------------- | --------------- | ---------------------------------------------- |
| Product release update | Public        | Safe for external sharing                       |
| Customer onboarding  | Public          | Must not reference unreleased capabilities      |
| Internal roadmap     | Internal        | Never generated from inference alone            |
| Investor update      | Internal        | Requires human review (see `overrides.yaml`)    |

## Accessibility requirements

Generated artifacts must carry text alternatives for every image, must not rely
on colour alone to convey state, and must keep captions available whenever a
video carries narration or meaningful audio.
