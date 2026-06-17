---
"fkst-hosted": minor
---

Deliver the GitHub App installation token to the substrate engine's `git` via an external credential helper, with rotation and expiry handling and zero changes to fkst-substrate (issue #107).

`git push` over HTTPS does not read `GITHUB_TOKEN`, so a goal session's `git` previously could not authenticate. The session now:

- writes the token file (`<runtime_dir>/github-token`) as JSON `{ "token": "ghs_…", "expires_at": "<RFC3339>" }` (mode `0600`), written atomically (tmp + rename) by both the startup write and the periodic/reactive/JIT refresh, via one shared writer so the format never diverges;
- materializes a `0700` credential helper (`git-credential-fkst`) into the runtime dir that reads the token file and answers `username=x-access-token` / `password=<token>`, with a just-in-time re-mint near expiry (it drops a nonce-authenticated request file the driver's poller services, then re-reads the rewritten file) and a stale-but-valid fallback so it never fails `git` hard;
- injects the git wiring on the engine process via platform-set `GIT_CONFIG_*` env (`credential.https://github.com.helper=!<helper>`, `useHttpPath=false`, `url.https://github.com/.insteadOf=git@github.com:`) — no on-disk `.git/config`, no token in any remote URL. These keys plus `GH_TOKEN` and `FKST_GITHUB_MINT_NONCE` are reserved so a user `env_profile` can never shadow them;
- hardens the periodic backstop: the refresh cooldown tightens as expiry approaches, and a persistent mint failure with an already-expired token (or an `InstallationGone`) transitions the session to a clear `Failed` state with a reason instead of letting substrate hit a silent 401.

`GITHUB_TOKEN`/`GH_TOKEN` are still set for `gh` as a frozen, best-effort convenience (it cannot rotate past the ~1h installation-token TTL — documented). The token value and the mint nonce are `SecretString` and are never logged.
