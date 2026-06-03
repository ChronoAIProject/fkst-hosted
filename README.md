# fkst-packages

这是官方 fkst 包库。库 B 只放可复用的官方脚本包，不承载 host 业务仓的状态，也不扩展引擎 surface。

每个官方公司放在 `packages/<name>/`，并作为一个独立 package-root 加载。package-root 的固定结构是：

- `departments/<department>/main.lua`
- `raisers/<raiser>.lua`
- 包内共享库直接放 package-root（如 `core.lua`），按 `require("core")` 引用；只做包内共享，不跨包 `require`
- `tests/*_test.lua` 或 `departments/*/*_test.lua`

加载示例：

```sh
fkst-framework run <department-main.lua> \
  --project-root <host-root> \
  --package-root /Users/auric/fkst-packages/packages/<name> \
  --event '<event-json>'
```

本机配置（`fkst-framework` 二进制路径等）放在库根的 `.env`（已 gitignore）。首次使用先从 `env.example` 复制并填好本机路径：

```sh
cp env.example .env   # 然后编辑 .env，把 BIN 指向你的 fkst-framework
```

集成测试 `packages/github-proxy/tests/integration.sh` 会自动加载库根 `.env`。常用测试命令（从库根运行）：

```sh
set -a; . ./.env; set +a   # 载入本机 BIN 等配置
"$BIN" test \
  --project-root "$PWD/packages/github-proxy" \
  --package-root "$PWD/packages/github-proxy"
```

本库不做 manifest、root-list、override DSL 或多包组合语言。引擎一次加载一个 `--package-root`，再叠加 host root，图由固定的 `departments/` 和 `raisers/` 目录扫描得到。

共享代码只在包内共享：共享库放 package-root（如 `core.lua`），被本包的 `departments/`、`raisers/` 按 `require("core")` 引用。不跨包 `require`——跨包引用会引入版本耦合，正是上面拒绝的多包组合。多个包都需要的通用、稳定能力应进引擎 SDK（像 `json.decode`），否则各包自带一份；宁可重复，不可耦合。

## github-proxy

`packages/github-proxy/` 是首个官方公司：GitHub ↔ fkst 事件桥，首切只覆盖 issue。

- 入站：`raisers/github_poll.lua` 每 5 分钟产生 `github_poll_tick`；`departments/github_poll/main.lua` 调用 host PATH 上的 `gh issue list`，把 GitHub issue 转成 `github_issue_seen`。
- 出站：`departments/github_comment/main.lua` 消费 host 注入的 `github_issue_comment_request`，默认 dry-run；只有 `FKST_GITHUB_WRITE=1` 时才会调用 `gh issue comment` 写回 GitHub。
- 去重：入站用 host git commit ledger，commit message 为 `github-proxy:seen:issue:<repo>#<number>@<updated_at>`。ledger commit 会写入 host repo 历史，作为 host fact；它只提交 `.fkst-github-proxy-ledger/` 专用路径，避免卷入 host 已 staged 的业务改动。如果 raise 后、commit 前崩溃，下次 tick 会再次 raise；下游应按 `dedup_key` 幂等。
- 注释幂等：写回评论时在 body 末尾附加 HTML marker，写前先读取现有 comments 并检查 marker。

配置由 host 提供：

- `FKST_GITHUB_REPO=owner/repo` 必填；缺失时 fail-closed。
- `FKST_GITHUB_WRITE=1` 可选；未设置时只 dry-run，不调用 mutate GitHub 的 `gh` 命令。
- `gh` auth、PATH、权限和 repo 当前 git 工作区都是 host 责任。

本包不会自动 supervise，也不会在测试中打真 GitHub。集成测试把 fake `gh` 放到 PATH 前面：

```sh
bash packages/github-proxy/tests/integration.sh
```
