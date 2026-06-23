# github-devloop Dogfood Topology

`github-devloop` dogfood uses one integration branch per device:

```text
develop feature branch
  -> PR to integration-<device>
  -> CI on integration-<device>
  -> rollup PR to dev after test success
  -> protected dev
```

`<device>` is the stable identity for that machine: the bot login used by that host, such as `ElonSG` or `loning`. The branch name is therefore `integration-<bot-login>`, for example `integration-ElonSG`.

中文补充：每台机器使用自己的 `integration-<device>` 测试分支，`<device>` 等于该机器的 bot login；`dev` 仍是受保护的最终集成分支。

## Host Config

The package does not derive or create the integration branch. Host env supplies the topology:

```sh
# github-devloop autonomous dogfood (per-device topology): develop -> integration-<device> -> rollup -> dev
# Values below are illustrative for a host whose bot login is ElonSG; substitute this host's identity.
FKST_GITHUB_REPO=ChronoAIProject/fkst-packages
FKST_GITHUB_WRITE=1
FKST_GITHUB_BOT_LOGIN=ElonSG                       # this host's bot login = the <device> identity
FKST_GITHUB_PROXY_POLL_LABEL_PREFIX=fkst-dev:       # let the generic adapter replay lifecycle-managed labels
FKST_DEVLOOP_UPSTREAM_BRANCH=dev
FKST_DEVLOOP_INTEGRATION_BRANCH=integration-ElonSG  # integration-<bot-login>, one per machine
FKST_DEVLOOP_ROLLUP_MERGE=auto
```

Keep `BIN`, `FKST_RUNTIME_ROOT`, `FKST_DURABLE_ROOT`, and `FKST_RATE_POOL_ROOT` configured as described in `.fkst/env.example` and the repository `README.md`.

## Second-Machine Bootstrap

Run these steps once on each additional machine:

1. Set the stable `DEVICE` value — that host's bot login (the same value used for
   `FKST_GITHUB_BOT_LOGIN`). The commands below use it as a shell variable, so set it first
   (replace `loning` with this host's bot login):

   ```sh
   DEVICE=loning
   ```

2. Create the per-device integration branch once from current `dev`. The push uses `FETCH_HEAD`,
   which the preceding `git fetch` sets to the just-fetched `dev` tip:

   ```sh
   git fetch origin dev
   git push origin "FETCH_HEAD:refs/heads/integration-$DEVICE"
   ```

3. Set the host env:

   ```sh
   export FKST_GITHUB_REPO=ChronoAIProject/fkst-packages
   export FKST_GITHUB_WRITE=1
   export FKST_GITHUB_BOT_LOGIN="$DEVICE"
   export FKST_GITHUB_PROXY_POLL_LABEL_PREFIX=fkst-dev:
   export FKST_DEVLOOP_UPSTREAM_BRANCH=dev
   export FKST_DEVLOOP_INTEGRATION_BRANCH="integration-$DEVICE"
   export FKST_DEVLOOP_ROLLUP_MERGE=auto
   ```

4. Launch `github-devloop` supervise from a worktree checked out to the merged `dev`, with the host's normal `BIN`, runtime, durable, and rate-pool env.

The `integration-<device>` branch must already exist before launch. By design, `github-devloop` holds instead of auto-creating a missing integration branch, because remote branch creation is an explicit host topology action.

## Transition Note

Keep the shared `integration` branch while any in-flight PR still targets it. Deleting a base branch closes its open PRs on GitHub, so check PR bases and other branch dependencies before deleting or changing any remote branch.

中文补充：迁移期间不要删除共享 `integration` 分支；先确认没有 open PR 以它为 base。

⟦AI:FKST⟧
