---
"fkst-hosted": minor
---

Trim the control plane to the v1 "one issue = one session" surface. Remove the
goal-submit, GitHub-proxy, Ornn-catalog, admin-state, and repo-scaffold routes
along with their datastore-era machinery (in-memory session/goal/vault stores,
the reconcile loop, the `github_hub` fan-out). The session endpoints are now
Kubernetes-backed: `GET /sessions/{owner}/{repo}/{issue}` returns rich live
status (pod id, start timestamp, fkst-substrate version, repo URL, issue
number, status) and `POST .../stop` deletes the backing Job (the pod and
per-session Secret cascade).
