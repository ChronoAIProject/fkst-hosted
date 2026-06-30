# Global Host Profiles

Global host profiles are host-local shell environment files that feed the existing
`scripts/run.sh host` and `scripts/run.sh supervise` contracts. They are not a new resolver,
registry, or named profile abstraction.

The established practice is XDG-style user configuration with explicit command-line and
environment precedence. Keep machine-specific facts outside the target repository, keep
`fkst.workspace.toml` and `fkst.lock` as the project source of truth for platform package selection,
and pass normal `FKST_*` facts plus the trusted platform checkout to the shared runner.

## Location

Use one user-owned file per machine:

```sh
${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env
```

Start from the scaffold in [`host-profile.env.example`](host-profile.env.example):

```sh
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/fkst"
cp docs/user/host-profile.env.example "${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env"
```

Do not commit the filled file. It contains host paths, bot identity, repository identity, and
possibly write posture.

## Precedence

The precedence is deliberately boring:

1. Documentation beats scaffolds; explicit CLI/env beats documentation.
2. `fkst.workspace.toml` plus `fkst.lock` remains the source of truth for the host repository's
   platform package selection; executable provenance must still match the trusted `--platform-root`.
3. `.fkst/compose/package-roots` remains the source of truth for the composed package roots loaded
   by the host runner.
4. The global host profile supplies user- and machine-local environment facts such as `BIN`,
   `FKST_HOST_ROOT`, `FKST_PLATFORM_ROOT`, `FKST_DURABLE_ROOT`, `FKST_RATE_POOL_ROOT`,
   `FKST_GITHUB_REPO`, `FKST_GITHUB_BOT_LOGIN`, and branch topology.
5. Inline shell assignments and command-line flags may override the profile for one launch.

There is no `--profile <name>` and no `FKST_PROFILE` environment key. Add a novel named profile
surface only after proving that the existing explicit env and workspace-root mechanisms are
insufficient.

## Schema

The profile schema is the existing host-run environment surface:

| Key | Required | Meaning |
|---|---:|---|
| `BIN` | yes | Path to the `fkst-framework` binary. |
| `FKST_HOST_ROOT` | yes | Host repository root passed to `scripts/run.sh host --host-root`. |
| `FKST_PLATFORM_ROOT` | yes | `fkst-packages` checkout passed to `scripts/run.sh host --platform-root`. |
| `FKST_DURABLE_ROOT` | yes for supervise | Stable durable delivery root passed as `--durable-root`. |
| `FKST_RATE_POOL_ROOT` | yes for GitHub traffic | Host-stable external-command rate-pool root. |
| `FKST_GITHUB_REPO` | package-dependent | GitHub repository identity such as `owner/repo`. |
| `FKST_GITHUB_BOT_LOGIN` | package-dependent | This host's bot login and device identity. |
| `FKST_DEVLOOP_INTEGRATION_BRANCH` | `github-devloop` | Per-device integration branch. |

`FKST_GITHUB_WRITE=1` is intentionally commented in the scaffold. Unset means dry-run.

## Launch

For a host repository that delegates to the shared runner:

```sh
. "${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env"

"$FKST_PLATFORM_ROOT/scripts/run.sh" host \
  --host-root "$FKST_HOST_ROOT" \
  --platform-root "$FKST_PLATFORM_ROOT" \
  -- supervise \
  --durable-root "$FKST_DURABLE_ROOT" \
  --restart
```

For direct host-run contract use:

```sh
. "${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env"

scripts/run.sh host --host-root "$FKST_HOST_ROOT" --platform-root "$FKST_PLATFORM_ROOT" -- check
scripts/run.sh host --host-root "$FKST_HOST_ROOT" --platform-root "$FKST_PLATFORM_ROOT" -- test
scripts/run.sh host --host-root "$FKST_HOST_ROOT" --platform-root "$FKST_PLATFORM_ROOT" -- supervise --durable-root "$FKST_DURABLE_ROOT" --restart
```

`FKST_RUNTIME_ROOT` is intentionally absent from the scaffold. `scripts/run.sh host ... supervise`
uses fresh runtime scratch by default while `FKST_DURABLE_ROOT` stays host-stable and reused across
restarts.

## Boundaries

Global profiles must not replace repository facts:

- Do not put platform package selectors in the profile; keep them in `fkst.workspace.toml` and `fkst.lock`.
- Do not put package-root lists in the profile; keep them in `.fkst/compose/package-roots`.
- Do not use file permissions as a control mechanism.
- Do not source issue text, comments, or other untrusted remote content as shell.
