# fkst-packages

这是官方 fkst 包库。库 B 只放可复用的官方脚本包，不承载 host 业务仓的状态，也不扩展引擎 surface。

每个官方公司放在 `packages/<name>/`。package-root 的固定结构是：

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

包库里有两类 package：

- **flat 平包**：自洽、自有裸名队列、0 外部 package namespace 引用，可单根 `conformance + test`。当前 flat 包有 `github-proxy` 与 `consensus`。
- **composed 包**：作为一等包放在 `packages/<name>/`，用于组合/适配兄弟包，department 可引用 `<pkg>.<queue>`；必须用 `composed.deps` 声明所组合的兄弟包，并通过组合 conformance 校验。当前 composed 包有 `autochrono`、`github-autochrono` 与 `github-devloop`。

引擎按 package-root 目录 basename 建命名空间。flat 包内队列写裸名；composed 包的 glue 队列按 `<pkg>.<queue>` 引用兄弟包。`composed.deps` 只是本仓测试组合的最小约定，不是版本解析、部署依赖或 override manifest。

本机配置（`fkst-framework` 二进制路径等）放在库根的 `.env`（已 gitignore）。首次使用先从 `env.example` 复制并填好本机路径：

```sh
cp env.example .env   # 然后编辑 .env，把 BIN 指向你的 fkst-framework
```

通用脚本 `scripts/run.sh`（从库根运行；自动解析 `fkst-framework` 二进制：`$BIN` > `.env` 的 `BIN=` > PATH > 同级 `../fkst-substrate`）：

```sh
scripts/run.sh test                 # self-test，所有包测试；flat 跑单根 conformance，composed 跳单根 conformance；最后跑组合 conformance；等价 CI
scripts/run.sh test github-proxy    # 只跑某个包；flat 跑 conformance + test
scripts/run.sh test-composed        # 只跑 composed 包及其递归 deps 的组合 conformance

# 通用一次性跑某部门：解码 RAISED 事件 + dump <RT> 树。包特定配置走 env。
# github-proxy 的只读入站 dogfood（拿真 gh 打真仓，不写 GitHub）：
FKST_GITHUB_REPO=ChronoAIProject/fkst-substrate scripts/run.sh run github-proxy github_poll

# 真实前台事件循环：脚本创建临时 FKST_RUNTIME_ROOT 和独立 FKST_DURABLE_ROOT。
# 可用 FKST_PROJECT_ROOT 覆盖默认 project-root（packages/<pkg>）。
FKST_GITHUB_REPO=owner/repo scripts/run.sh supervise github-proxy

# 本地 test/run/supervise 会对可溯源到 ../fkst-substrate 的 BIN 做 freshness 自动构建；
# CI 不自动 build，FKST_NO_AUTOBUILD=1 可跳过。显式 build 仍会 git pull && cargo build。
scripts/run.sh build
```

`run` 用临时（或复用已设的）`FKST_RUNTIME_ROOT`、绝不设 `FKST_GITHUB_WRITE`，所以只读 dogfood 保持只读；同一 `FKST_RUNTIME_ROOT` 连跑两次可看去重。脚本对任何 `packages/<pkg>/departments/<dept>` 通用，不写死 github-proxy。

`supervise` 是真实 `fkst-framework supervise` 的薄封装，不搭 host harness、不模拟事件、不注入 fake `gh`；它在前台运行，按 `Ctrl-C` 退出。脚本会显式传 `--project-root`、`--package-root` 和 `--framework-bin`，并设置彼此不同的临时 `FKST_RUNTIME_ROOT` / `FKST_DURABLE_ROOT`。

本库不做版本化 manifest、root-list 或 override DSL。图由固定的 `departments/` 和 `raisers/` 目录扫描得到。flat 包可独立加载；composed 包显式承担跨包 wiring，并用 `composed.deps` 告诉测试脚本组合 conformance 需要一起加载哪些兄弟包。

共享代码只在包内共享：共享库放 package-root（如 `core.lua`），被本包的 `departments/`、`raisers/` 按 `require("core")` 引用。不跨包 `require`。跨包组合只通过事件队列契约连接；唯一同时引用 `github-proxy.*` 与 `autochrono.*` 的代码在 `packages/github-autochrono/`。多个包都需要的通用、稳定能力应进引擎 SDK（像 `json.decode`），否则各包自带一份；宁可重复，不可耦合。

## 测试约定

- unit：`tests/*_test.lua` 测纯函数/逻辑，用 `fkst.test` 的 `eq` / `is_true` / `raises` / `is_nil`。
- integration：`tests/*_test.lua` 用 `fkst.test.run_department(path, event, opts)` 测 department 端到端行为：注入事件、断言 raise，并用 `opts.env` 提供 `FKST_RUNTIME_ROOT` 等 host facts；外部 CLI 一律用引擎 test-mode `fkst.test.mock_command` / `fkst.test.command_calls` mock 和断言，不放 fake binary 到 PATH。未 mock 的外部命令会 fail-closed。
- 图布线与静态声明（raisers / 队列匹配）由 `fkst-framework conformance` 校验，不为静态声明写单测。
- flat 包：`scripts/run.sh test [pkg]` 对 flat 包跑单根 `conformance + test`。
- composed 包：`scripts/run.sh test [pkg]` 跳过单根 conformance，但仍跑该包 tests；无参全包测试收尾会跑组合 conformance。
- 组合 conformance：`scripts/run.sh test-composed` 收集所有带 `composed.deps` 的包及其递归依赖，以仓库根为 `--project-root`、收集到的包为 `--package-root` 验证 union graph。
- CI 调 `scripts/run.sh test`，与本地标准测试走同一路径：先跑一次 `fkst-framework --self-test`，flat 包跑 `conformance + test`，composed 包跑 test，最后跑组合 conformance。
- 新包清单：有逻辑就写 unit，有运行时行为就写 integration，布线靠 conformance。

## github-proxy

`packages/github-proxy/` 是首个官方公司：GitHub ↔ fkst 事件桥，覆盖 issue 与 PR。

- 入站：`raisers/github_poll.lua` 每 5 分钟产生 `github_poll_tick`；`departments/github_poll/main.lua` 调用 host PATH 上的 `gh issue list --state all` 和 `gh pr list --state all`，把 GitHub issue / PR 转成统一的 `github_entity_changed`，因此 close / merge 等最终状态转换也会浮出。事件包含 labels 快照，形如 `labels = {"fkst-dev:enabled", ...}`。
- 出站：`departments/github_comment/main.lua` 消费 host 注入的 `github_issue_comment_request`；`departments/github_issue_label/main.lua` 消费 `github_issue_label_request` 并调用 `gh issue edit --add-label/--remove-label`。两者默认 dry-run；只有 `FKST_GITHUB_WRITE=1` 时才会写回 GitHub。
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

本包不会自动 supervise，也不会在测试中打真 GitHub。Lua 集成测试用 `fkst.test.mock_command` mock `gh issue list` / `gh pr list` / `gh issue view` / `gh issue comment` / `gh issue edit`，并用 `fkst.test.command_calls` 断言发出的命令；不生成 fake `gh` 二进制。测试由 `fkst-framework test` 自动运行：

```sh
scripts/run.sh test github-proxy
```

## autochrono

`packages/autochrono/` 是组合 `consensus` 的 composed 包，负责把自有 issue 协议接到通用共识引擎。它不直接依赖 `github-proxy`，也不直接调用 `codex`；起草与多角度判断都由 `consensus` 承担。

- `departments/propose/main.lua` 消费裸名 `issue`，只处理 open issue，把 `autochrono.issue.v1` 映射为 `consensus.proposal.v1`，raise 到 `consensus.proposal`。
- `departments/reply/main.lua` 消费 `consensus.consensus_reached`。`decision = "approve"` 时产出裸名 `reply`；`decision = "reject"` 或 foreign proposal 静默跳过。
- 自有输入 schema 是 `autochrono.issue.v1`，自有输出 schema 是 `autochrono.reply.v1`；跨包链路是 `autochrono.issue -> consensus.proposal -> consensus.consensus_reached -> autochrono.reply`。
- `proposal_dedup_key(repo, issue_number, updated_at)` 按 issue update 版本化，避免同一 issue 的新内容被旧 proposal cache 吞掉；`reply_dedup_key(repo, issue_number)` 稳定为 `autochrono:<repo>#issue/<number>`，不含 `updated_at`。
- 防循环靠 `with_lock` + `cache_get/cache_set`；即使 runtime cache 丢失，稳定 `dedup_key` 仍会由 `github-proxy` 评论 HTML marker 做外部 durable 幂等。
- `composed.deps` 声明它需要把 `consensus` 一起加载做组合 conformance。测试保持 `autochrono` 零 `codex` 调用；涉及 `codex exec` 的 mock 留在 `consensus` 包内。

## consensus

`packages/consensus/` 是通用、多角度的 flat 共识引擎，不绑定 GitHub 或 autochrono。它消费抽象 `proposal`，在一个 pipeline 内启动多个 `codex exec` 角度；全体角度一致时产出 `consensus_reached`，无法达成一致或某角度失败/不可解析时产出 bounded `consensus_unresolved`。

- 输入 schema 是 `consensus.proposal.v1`，输出 schema 是 `consensus.consensus_reached.v1` 或 `consensus.consensus_unresolved.v1`。`consensus_unresolved` 只带 `schema`、`proposal_id`、`dedup_key`、`source_ref`，不带 body 或 angle 文本。
- department 内只消费/产生裸名队列；被 composed 包引用时，对外表现为 `consensus.proposal`、`consensus.consensus_reached` 与 `consensus.consensus_unresolved`。
- 可靠投递事件携带 `source_ref`，下游据此回源 derive 当前事实，不把 proposal payload 当跨 pipeline 真相。

## github-autochrono

`packages/github-autochrono/` 是组合 `github-proxy` + `autochrono` 的 composed 包，是本仓 CI 覆盖的一等 package。它只做适配/wiring，不承载起草或共识业务逻辑；`autochrono` 再通过自己的 `composed.deps` 组合 `consensus`。链路是：

```text
github-proxy.github_entity_changed
  -> autochrono.issue
  -> consensus.proposal
  -> consensus.consensus_reached
  -> autochrono.reply
  -> github-proxy.github_issue_comment_request
```

`github-proxy` 与 `autochrono` 互不认识；这个 composed glue 是唯一同时引用 `github-proxy.*` 与 `autochrono.*` 的层。入站 glue 只把 GitHub issue 事件转成 `autochrono.issue.v1`，出站 glue 把 `autochrono.reply.v1` 转成 GitHub 评论请求。`github-autochrono/composed.deps` 声明它需要把 `github-proxy` 与 `autochrono` 一起加载；测试脚本会递归带上 `autochrono` 依赖的 `consensus`。

组合 conformance 跑法：

```sh
fkst-framework conformance \
  --project-root /Users/auric/fkst-packages \
  --package-root /Users/auric/fkst-packages/packages/github-autochrono \
  --package-root /Users/auric/fkst-packages/packages/github-proxy \
  --package-root /Users/auric/fkst-packages/packages/autochrono \
  --package-root /Users/auric/fkst-packages/packages/consensus
```

## github-devloop

`packages/github-devloop/` 是组合 `github-proxy` + `consensus` 的 composed 包。它覆盖 issue design consensus、stuck meta-escalation 与 ready-to-implement：带 `fkst-dev:enabled` 且最新 GitHub 评论里没有本 proposal 的 `fkst:github-devloop:state:v1` marker 的 issue 被回源读取 body，映射成 `consensus.proposal`，并先写 `thinking` state marker；当 `consensus.consensus_reached` 返回 `approve` / `reject` 时，只有最新 state marker 仍是 `thinking` 才转到 `ready` / `blocked`，发一条带新 state marker 与 `fkst:github-devloop:result:v1` marker 的评论作为外部 durable 记录；`approve` 同时产生包内裸名 `devloop_ready` 事件（schema `github-devloop.ready.v1`）。

当 `consensus.consensus_unresolved` 到达时，`departments/loop` 回源读取当前 issue body 与评论 marker：最新 state marker 仍是 `thinking` 且未达到预算时，重建并重新 raise `consensus.proposal`，同时请求写入 `fkst:github-devloop:loop:v1` marker；达到预算时写入 `stuck` state marker 与 `fkst:github-devloop:stuck:v1` marker，并产生包内裸名 `devloop_stuck` 事件（schema `github-devloop.stuck.v1`）。

`departments/meta` 消费 `devloop_stuck`，回源确认最新 state marker 仍是 `stuck` 后，运行一次 meta `codex exec`，只接受严格相邻的 `⟦FKST:ACTION⟧ implement|split|block` 与 `⟦FKST:REASON⟧ ...` 输出。meta 结果评论里的 `fkst:github-devloop:meta:v1` marker 与 `github-proxy` comment dedup 只按当前 stuck version 幂等，不按 action 幂等：同一 `devloop_stuck` 可靠重放即使重新跑出不同 action，也只能落下第一条当前版本 meta 事实 marker。`implement` 写入 `ready` state marker 并产生 `devloop_ready`，`split` / `block` 写入 `blocked` state marker；`split` 只在 marker 评论里记录建议，不自动创建 GitHub issue。若最新 state marker 已进入 `implementing` 或 `impl-failed`，meta 按已推进或实现失败终态跳过，不再升级。

`departments/implement` 消费 `devloop_ready`，回源确认最新可信 state marker 仍是 `ready` 后，用这个 ready-CAS 门控一次实施尝试：调用 SDK `setup_worktree("devloop-" .. safe_issue_slug)` 创建隔离 git worktree，并用 `spawn_codex_sync({worktree=...})` 在该 worktree 内实施；ready-CAS gates the attempt，失败时写入 `impl-failed` state marker 与 failure marker；Codex 与 `git -C <worktree> status --porcelain` 成功且有变更时，写入当前版本、记录 worktree path 的 `implementing` state marker。Codex 非零退出或 status 为空会写入 `impl-failed` state marker 与 failure marker。Phase 3 不 push、不开 PR；Phase 4 才处理 PR。所有状态转移都以“最新可信 state marker + version CAS”为事实门：只信 `FKST_GITHUB_BOT_LOGIN` 对应 bot 作者的 marker，当前等于目标状态即幂等跳过，当前尚未到达前置状态则 error 触发可靠投递重试，incoming version 旧于当前 marker version 或当前已推进/分叉则跳过旧 replay。GitHub 是 eventually-consistent fact source，不是 strong-consistency KV；同 issue transition 共用同一个 `with_lock` key，marker 写入按 dedup 幂等，每次投递回源重导并自愈，读-CAS 到异步写 marker 的小 race window按最终一致性收敛。`fkst-dev:<state>` label 只是 set-exclusive、best-effort、自愈的 UI hint：每次转移都会请求加目标状态 label 并移除其他状态 label，但 correctness 不依赖 label，`github-proxy` 不再做 label 缺席预检。所有 label/comment 写入都经 `github-proxy` 的 dry-run-by-default 出站队列。`github-devloop/composed.deps` 声明它需要把 `github-proxy` 与 `consensus` 一起加载做组合 conformance。
