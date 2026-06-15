---
"fkst-hosted": minor
---

Install the `nyxid` CLI (pinned v0.7.0) into the runtime pod image via a new `cli-builder` multi-stage that clones NyxID at the pinned tag and builds the `nyxid-cli` workspace member, then copies the `nyxid` binary to `/usr/local/bin` and asserts `nyxid --version` reports v0.7.0 at build. This lets a fkst-package's business logic shell out to `nyxid …` as the session user (the per-session `NYXID_ACCESS_TOKEN` + `NYXID_URL` it authenticates with are provisioned by the session-token work; the CLI resolves its base URL from `NYXID_URL`). No Rust changes.
