# 新建 fkst package-repo bootstrap 清单

这份清单用于把一个新仓库搭成可复制的 fkst package-repo scaffold。权威契约以 `fkst-substrate/docs/package-repo-contract.md` 为准；本文只列最小落地步骤。

1. Create the runtime interface directory `.fkst/` with tracked `.fkst/env.example` and `.fkst/substrate-ref`; ignore `.fkst/packages`, `.fkst/local-packages`, `.fkst/run/`, and `.fkst/env`. Everything else under `.fkst/` is runtime-generated.
2. For a Lua-primary package repo, keep committed development source in `packages/<pkg>/...`. `scripts/run.sh` regenerates `.fkst/local-packages -> ../packages` as the own-package runtime view. `.fkst/packages/<pkg>/...` is only for external referenced packages assembled by an operator or dogfood host; it is empty for a package library itself.
3. 创建包目录：`packages/<pkg>/core.lua`、`packages/<pkg>/departments/<dept>/main.lua`、按需 `packages/<pkg>/raisers/<raiser>.lua`、`packages/<pkg>/tests/*_test.lua`。
4. 如果是 composed package，在 `packages/<pkg>/fkst.toml` 写 `kind = "package.composed"`，并在 `[event_deps] packages = [...]` 声明需要一起加载做组合 conformance 的兄弟包名；flat package 用 `kind = "package"` 且不要声明 `[event_deps]`。
5. 从 scaffold 复制 `scripts/run.sh`、`scripts/check_repo.py`、`.fkst/env.example` 和 `.github/workflows/ci.yml`。
6. 在 `.fkst/substrate-ref` 写入 source pin。默认值可用 `dev`；下游可复现仓库应改成 `fkst-substrate` 的 tag 或 SHA。这是 Git source-pin，不是 semver，也不是二进制分发。
7. For a local `fkst-framework` binary cache path keyed by an explicit source pin, use the pure helper `substrate_bin_cache_path(cache_root, owner, repo, ref)` from `scripts/bin_cache.py`. Path contract: `<cache_root>/fkst-substrate-bin/v1/<owner>/<repo>/<ref>/target/debug/fkst-framework`, with `owner`, `repo`, and `ref` encoded as independent UTF-8 byte percent-encoded path components. `/`, space, `.`, `..`, `%`, and other special characters stay data, not separators or dot-segments, so distinct `(owner, repo, ref)` triples cannot collide through separator replacement. `scripts/run.sh` uses that path only after all ordinary `BIN` sources miss, then serializes clone/fetch/checkout/build with a per-cache lock. Invalid explicit `BIN` or `.fkst/env BIN=` fails closed instead of falling back. `FKST_NO_AUTOBUILD=1` disables the network/build fallback.
8. 复制 `.fkst/env.example` 为 `.fkst/env`，设置 `BIN=/path/to/fkst-substrate/target/debug/fkst-framework`；CI 会自己从 `.fkst/substrate-ref` checkout engine source 并 build `fkst-framework`。
9. 从仓库根运行 `scripts/run.sh test`。无参运行会先执行 `fkst-framework --self-test`，再从 `packages/*` 枚举要测试的 own packages；引擎实际加载 `.fkst/local-packages/<pkg>`，并在组合 / run / supervise 场景额外包含 `.fkst/packages/*` 中存在的 external roots。对 flat package 跑单包 `conformance + test`，对 composed package 跳过单包 conformance 但仍跑 test，最后按所有 `fkst.toml` `[event_deps]` 递归做组合 conformance。未设置时 `scripts/run.sh` 使用 `.fkst/run/runtime` 和 `.fkst/run/durable`；board cache 写到 `.fkst/run/board-cache.json`。
10. 本地需要只跑静态仓库守卫时用 `scripts/run.sh check`；需要只跑组合 conformance 时用 `scripts/run.sh test-composed`。

新增包时保持 payload 小而稳定：可靠投递只放 `source_ref`、`schema`、`dedup_key` 和控制字段；大体量 issue body、PR diff、评论、代码或文件内容由 consumer 通过 `source_ref` 回源读取。

⟦AI:FKST⟧
