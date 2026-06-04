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

通用脚本 `scripts/integration.sh`（从库根运行；自动解析 `fkst-framework` 二进制：`$BIN` > `.env` 的 `BIN=` > PATH > 同级 `../fkst-substrate`）：

```sh
scripts/integration.sh test                 # 跑所有包的 *_test.lua（test 模式，等价 CI）
scripts/integration.sh test github-proxy    # 只跑某个包

# 通用一次性跑某部门：解码 RAISED 事件 + dump <RT> 树。包特定配置走 env。
# github-proxy 的只读入站 dogfood（拿真 gh 打真仓，不写 GitHub）：
FKST_GITHUB_REPO=ChronoAIProject/fkst-substrate scripts/integration.sh run github-proxy github_poll
```

`run` 用临时（或复用已设的）`FKST_RUNTIME_ROOT`、绝不设 `FKST_GITHUB_WRITE`，所以只读 dogfood 保持只读；同一 `FKST_RUNTIME_ROOT` 连跑两次可看去重。脚本对任何 `packages/<pkg>/departments/<dept>` 通用，不写死 github-proxy。

本库不做 manifest、root-list、override DSL 或多包组合语言。引擎一次加载一个 `--package-root`，再叠加 host root，图由固定的 `departments/` 和 `raisers/` 目录扫描得到。

共享代码只在包内共享：共享库放 package-root（如 `core.lua`），被本包的 `departments/`、`raisers/` 按 `require("core")` 引用。不跨包 `require`——跨包引用会引入版本耦合，正是上面拒绝的多包组合。多个包都需要的通用、稳定能力应进引擎 SDK（像 `json.decode`），否则各包自带一份；宁可重复，不可耦合。

## 测试约定

- unit：`tests/*_test.lua` 测纯函数/逻辑，用 `fkst.test` 的 `eq` / `is_true` / `raises` / `is_nil`。
- integration：`tests/*_test.lua` 用 `fkst.test.run_department(path, event, opts)` 测 department 端到端行为：注入事件、断言 raise，并用 `opts.env` / `path_prepend` 提供 `FKST_RUNTIME_ROOT` / 假命令。
- 图布线与静态声明（raisers / 队列匹配）由 `fkst-framework conformance` 校验，不为静态声明写单测。
- CI 对每个包跑 `fkst-framework test`（+ conformance）。
- 新包清单：有逻辑就写 unit，有运行时行为就写 integration，布线靠 conformance。

## github-proxy

`packages/github-proxy/` 是首个官方公司：GitHub ↔ fkst 事件桥，覆盖 issue 与 PR。

- 入站：`raisers/github_poll.lua` 每 5 分钟产生 `github_poll_tick`；`departments/github_poll/main.lua` 调用 host PATH 上的 `gh issue list --state all` 和 `gh pr list --state all`，把 GitHub issue / PR 转成统一的 `github_entity_changed`，因此 close / merge 等最终状态转换也会浮出。
- 出站：`departments/github_comment/main.lua` 消费 host 注入的 `github_issue_comment_request`，默认 dry-run；只有 `FKST_GITHUB_WRITE=1` 时才会调用 `gh issue comment` 写回 GitHub。
- 入站缓存：每个实体用可读路径 key `github-proxy/<type>/<repo>/<num>` 读写引擎 `cache_get` / `cache_set`，例如 `github-proxy/issue/owner/repo/42`。缓存值只保存最新 `updated_at` 并覆盖写入，因此不会积累 marker。
- 变更检测：poll 到的新 `updated_at` 与缓存不同就先 raise `github_entity_changed`，再 `cache_set`。事件包含 `schema`、`type`（`issue` 或 `pr`）、`repo`、`number`、`title`、`url`、`state`、`updated_at`、`dedup_key`、`source`，以及 `source_ref`（`{kind="external", ref="<repo>#<type>/<number>"}`）。如果 raise 后、写缓存前崩溃，下次 tick 会再次 raise 同一个 `dedup_key`；下游按 `dedup_key` 幂等。
- 对齐 substrate 的持久投递引擎：`source_ref` 是稳定的实体指针，可靠消费者据此**回源 derive 当前实体**（如 `gh issue view`）而非信任可能过期的 payload；事件被路由到可靠订阅时引擎也要求带它。`payload` 里的实体字段是 best-effort 快照、便于轻量消费。真实运行（`fkst-framework supervise`）需配 `FKST_DURABLE_ROOT`（见 `env.example`）。
- 轮询窗口：list polling 受 `gh` 默认返回数量限制；窗口外的实体可能不会被本轮重新检查。这是 best-effort 入站信号，下游应从 durable GitHub state 重新推导最终状态。
- 并发：每个实体更新都包在 `with_lock("github-proxy/<type>/<repo>/<num>", fn)` 内，避免同一实体的 cache 比较和写入交错。
- 注释幂等：写回评论时在 body 末尾附加 HTML marker，写前先读取现有 comments 并检查 marker。

配置由 host 提供：

- `FKST_GITHUB_REPO=owner/repo` 必填；缺失时 fail-closed。
- `FKST_RUNTIME_ROOT=/path/to/runtime` 必填；引擎用它管理 cache / lock 状态，缺失时入站 poll fail-closed。
- `FKST_GITHUB_WRITE=1` 可选；未设置时只 dry-run，不调用 mutate GitHub 的 `gh` 命令。
- `gh` auth、PATH、权限和 repo 当前 git 工作区都是 host 责任。

本包不会自动 supervise，也不会在测试中打真 GitHub。Lua 集成测试把 fake `gh` 放到 PATH 前面，并由 `fkst-framework test` 自动运行：

```sh
"$BIN" test \
  --project-root "$PWD/packages/github-proxy" \
  --package-root "$PWD/packages/github-proxy"
```
