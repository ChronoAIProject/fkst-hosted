# 新建 fkst package-repo bootstrap 清单

这份清单用于把一个新仓库搭成可复制的 fkst package-repo scaffold。权威契约以 `fkst-substrate/docs/package-repo-contract.md` 为准；本文只列最小落地步骤。

1. 创建包目录：`packages/<pkg>/core.lua`、`packages/<pkg>/departments/<dept>/main.lua`、按需 `packages/<pkg>/raisers/<raiser>.lua`、`packages/<pkg>/tests/*_test.lua`。
2. 如果是 composed package，在 `packages/<pkg>/composed.deps` 逐行写入需要一起加载做组合 conformance 的兄弟包名；flat package 不要创建 `composed.deps`。
3. 从 scaffold 复制 `scripts/run.sh`、`scripts/check_repo.py`、`env.example` 和 `.github/workflows/ci.yml`。
4. 在仓库根目录创建 `.fkst-substrate-ref`。默认值可用 `dev`；下游可复现仓库应改成 `fkst-substrate` 的 tag 或 SHA。这是 Git source-pin，不是 semver，也不是二进制分发。
5. For a local `fkst-framework` binary cache path keyed by an explicit source pin, use the pure helper `substrate_bin_cache_path(cache_root, owner, repo, ref)` from `scripts/bin_cache.py`. Path contract: `<cache_root>/fkst-substrate-bin/v1/<owner>/<repo>/<ref>/target/debug/fkst-framework`, with `owner`, `repo`, and `ref` encoded as independent UTF-8 byte percent-encoded path components. `/`, space, `.`, `..`, `%`, and other special characters stay data, not separators or dot-segments, so distinct `(owner, repo, ref)` triples cannot collide through separator replacement. Non-goals: this helper does not parse pins, checkout, clone, build, use the network, or change `scripts/run.sh` `BIN` resolution. 中文补充：这个 helper 只定义 collision-free cache path contract。
6. 复制 `env.example` 为 `.env`，设置 `BIN=/path/to/fkst-substrate/target/debug/fkst-framework`；CI 会自己从 `.fkst-substrate-ref` checkout engine source 并 build `fkst-framework`。
7. 从仓库根运行 `scripts/run.sh test`。无参运行会先执行 `fkst-framework --self-test`，再枚举 `packages/*`，对 flat package 跑单包 `conformance + test`，对 composed package 跳过单包 conformance 但仍跑 test，最后按所有 `composed.deps` 递归做组合 conformance。
8. 本地需要只跑静态仓库守卫时用 `scripts/run.sh check`；需要只跑组合 conformance 时用 `scripts/run.sh test-composed`。

新增包时保持 payload 小而稳定：可靠投递只放 `source_ref`、`schema`、`dedup_key` 和控制字段；大体量 issue body、PR diff、评论、代码或文件内容由 consumer 通过 `source_ref` 回源读取。

⟦AI:FKST⟧
