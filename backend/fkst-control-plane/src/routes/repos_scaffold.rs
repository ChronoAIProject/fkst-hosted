//! Static `.fkst/` scaffold content committed by `POST /repos/:owner/:name/
//! fkst-setup` (#181).
//!
//! These are compile-time, non-sensitive defaults — they NEVER contain a secret,
//! an env value, or a goal prompt. The example package satisfies the engine's
//! #115 conformance contract (`crate::engine::materialize::PreparedPackage::validate`):
//! it is named `example` and carries the anchored entry file
//! `departments/example/main.lua`. A unit test in `routes::repos` rebuilds the
//! package from these consts and asserts `validate()` succeeds, guarding the
//! scaffold against ever drifting out of conformance.

use crate::github_hub::ScaffoldFile;

/// Repo-relative path of the example department entry file.
pub const EXAMPLE_MAIN_LUA_PATH: &str = ".fkst/packages/example/departments/example/main.lua";
/// Repo-relative path of the example package README.
pub const EXAMPLE_README_PATH: &str = ".fkst/packages/example/README.md";
/// Repo-relative path of the per-repo base codex instruction file.
pub const AGENTS_MD_PATH: &str = ".fkst/AGENTS.md";

/// The package-root-relative entry path (NOT prefixed with `.fkst/packages/
/// example/`), used only to rebuild a `PreparedPackage` in the conformance test.
pub const EXAMPLE_ENTRY_RELATIVE: &str = "departments/example/main.lua";
/// The example package's name (must match `^[A-Za-z0-9_-]+$`, not `host`).
pub const EXAMPLE_PACKAGE_NAME: &str = "example";

/// Minimal conformant department entry — an empty module table, the shape the
/// engine accepts as a valid department (#115). Replace the body with real
/// department logic.
pub const EXAMPLE_MAIN_LUA: &str = "\
-- Example fkst department entry: returns an empty module table.
-- This is the minimal package the engine accepts (see #115); replace the body
-- with your real department logic.
local M = {}
return M
";

/// README explaining the example package.
pub const EXAMPLE_README_MD: &str = "\
# Example fkst package

This is the example fkst package named `example`, written by `fkst-hosted`'s
repo-setup endpoint to bootstrap this repository for fkst.

## Layout

```
.fkst/packages/example/
  departments/
    example/
      main.lua   <- the engine entry point for the `example` department
  README.md      <- this file
```

The engine entry point is `departments/example/main.lua`. Every fkst package
needs at least one `departments/<name>/main.lua`; `main.lua` returns a Lua
module table that the engine loads.

## Adding real departments

Add another `departments/<your-department>/main.lua` (and any supporting Lua
files alongside it). Keep each department self-contained.

## Using this package in a goal

Reference the package by its directory name (`example`) in a goal's
`package_names`. fkst resolves each name against this repo's
`.fkst/packages/<name>/` directory at session start.
";

/// The default per-repo base codex instruction file. Generic on purpose: it
/// documents WHAT the file is, not project-specific behavior.
pub const DEFAULT_AGENTS_MD: &str = "\
# AGENTS.md — per-repo fkst base instructions

This file is the **base instruction set prepended to every fkst session spawned
from this repository**. It is read by the coding agent (codex) — hence the
uppercase `AGENTS.md` name — and `fkst-hosted` injects its contents ahead of the
goal-specific prompt at session start.

Edit this file to give every session in this repo shared context: coding
conventions, architectural constraints, the layout of the codebase, what to
avoid, and any house rules. Keep it concise and durable — it applies to *all*
sessions, so put goal-specific detail in the goal itself, not here.

> Note: the session-time injection of this file is handled by fkst-hosted; this
> file only needs to contain the instructions you want every session to see.
";

/// Assemble the three scaffold files in committed order: example entry, example
/// README, then the per-repo AGENTS.md.
pub fn scaffold_files() -> Vec<ScaffoldFile> {
    vec![
        ScaffoldFile {
            path: EXAMPLE_MAIN_LUA_PATH.to_string(),
            contents: EXAMPLE_MAIN_LUA.as_bytes().to_vec(),
        },
        ScaffoldFile {
            path: EXAMPLE_README_PATH.to_string(),
            contents: EXAMPLE_README_MD.as_bytes().to_vec(),
        },
        ScaffoldFile {
            path: AGENTS_MD_PATH.to_string(),
            contents: DEFAULT_AGENTS_MD.as_bytes().to_vec(),
        },
    ]
}

/// The repo-relative paths the scaffold writes, in committed order. Returned in
/// the `created_paths` response field.
pub fn scaffold_paths() -> Vec<String> {
    scaffold_files().into_iter().map(|f| f.path).collect()
}
