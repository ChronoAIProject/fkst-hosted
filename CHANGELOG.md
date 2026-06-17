# Changelog

All notable, user-facing changes to **fkst-hosted** are recorded here, latest
release on top. Each `## vX.Y.Z` section is the release notes for that version.

This file is maintained automatically by the release pipeline: the pending
section is written from the release note as part of every `develop → main`
release PR, and frozen into a permanent entry once the release is tagged. Do not
edit released sections by hand.

<!-- BEGIN PENDING -->
## v0.2.1 — 2026-06-17

## Fixed

- Few technical bugs fixed

## New Feature

## Changed

- Technical enhancement
<!-- END PENDING -->

## v0.2.0 — 2026-06-17

## Fixed

- Worker pods start without an explicit pod-identity env var
- Few technical bugs fixed

## New Feature

- OpenAPI 3 specification now served at `/openapi.json`

## Changed

- Technical enhancement

## v0.1.0 — 2026-06-17

## Fixed

- Goal runs now reach the engine with valid GitHub credentials
- Sessions now reliably reach the running state
- Clearer errors when a GitHub connection lacks repo scope
- Few technical bugs fixed

## New Feature

- Submit a goal from a GitHub issue or inline form
- Trigger agent runs on the fkst-substrate engine
- Optionally create a goal's GitHub repository for you
- Per-session secret and variable vault for runs
- Inject user-pinned Ornn skills into each run
- Pre-flight validation before a run starts
- Goal-issue lifecycle labels and an activity timeline
- Browse your GitHub issues across linked accounts
- Admin observability endpoints and metrics
- React web console for goals, packages, and settings
- Technical enhancement

## Changed

- Removed MongoDB; state lives in GitHub and memory
- Engine execution runs on an autoscaling worker fleet
- Packages are now repo-scoped under `.fkst/packages`
- Kubernetes-only deployment; docker-compose removed
- Authentication trusts the NyxID proxy with `fkst:*` permissions
- Technical enhancement
