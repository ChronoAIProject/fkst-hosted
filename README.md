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
# FKST_RATE_POOL_ROOT 必须由 host 设置为所有 supervise 实例共享的绝对路径。
# 可用 FKST_PROJECT_ROOT 覆盖默认 project-root（packages/<pkg>）。
FKST_GITHUB_REPO=owner/repo FKST_RATE_POOL_ROOT=/var/lib/fkst/rate-pools scripts/run.sh supervise github-proxy

# 本地 test/run/supervise 会对可溯源到 ../fkst-substrate 的 BIN 做 freshness 自动构建；
# CI 不自动 build，FKST_NO_AUTOBUILD=1 可跳过。显式 build 仍会 git pull && cargo build。
scripts/run.sh build
```

`run` 用临时（或复用已设的）`FKST_RUNTIME_ROOT`、绝不设 `FKST_GITHUB_WRITE`，所以只读 dogfood 保持只读；同一 `FKST_RUNTIME_ROOT` 连跑两次可看去重。脚本对任何 `packages/<pkg>/departments/<dept>` 通用，不写死 github-proxy。

`supervise` 是真实 `fkst-framework supervise` 的薄封装，不搭 host harness、不模拟事件、不注入 fake `gh`；它在前台运行，按 `Ctrl-C` 退出。脚本会显式传 `--project-root`、`--package-root` 和 `--framework-bin`，并设置彼此不同的临时 `FKST_RUNTIME_ROOT` / `FKST_DURABLE_ROOT`。真实 supervise 还会 fail-closed 要求 `FKST_RATE_POOL_ROOT` 是 host 提供的绝对路径；每个消耗同一 GitHub 配额的实例必须指向同一个目录。

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
- Boundary tests for truncation and cache-key generation must use package-local production-shaped fixture helpers. Fixtures should include multibyte CJK/emoji titles and full-length PR-review proposal ids, so byte boundaries and the `max_key_len=200` contract are reachable in tests instead of only in production.

## 测试运行体系 / 守卫

`scripts/run.sh check` 只运行仓库静态守卫，不解析也不执行 `BIN`。`scripts/run.sh test` 与 `scripts/run.sh test-composed` 会先运行这些守卫，再进入引擎真实测试；CI 仍只调用 `scripts/run.sh test`。

静态守卫由 `scripts/check_repo.py` 实现，只扫描 `packages/` 与 `scripts/`，不会广泛扫描仓库根，也不会扫 CI checkout 到根目录的 `fkst-substrate/`。这是快速、密闭、best-effort 的静态 lint，只负责不需要引擎的仓库形状约束；它不是完整 Lua parser，也不判断引擎实际会从 returned table 顶层枚举哪些 `test_` key。这个事实由下方 G5 的引擎真实输出检查负责；更深的 engine-loader-based Lua audit 已列入 engine-PR backlog。当前静态守卫包括：

- `packages/` 与 `scripts/` 下的 `.lua`、`.sh`、`.py`、`.rs` 源文件硬上限 1000 行；超过即失败，先按职责拆分或删除重复代码。
- `packages/*/tests/` 下的 Lua 文件只能命名为 `*_test.lua` 或 `*_helpers.lua`。
- 对 `*_test.lua` 做 best-effort “看起来有 `test_<name> = function` 定义”的提示；这只是早期 lint，不作为引擎枚举真相。
- 对单个 `*_test.lua` 内 best-effort 识别到的 top-level `test_<name>` key 做重复检查；常见 assignment form（如 `test_x = ...`、`["test_x"] = ...`、`M.test_x = ...`、`M["test_x"] = ...`）与常见 function-definition form（如 `function test_x() ... end`、`function M.test_x() ... end`、`function M:test_x() ... end`）都会归一到同一个 key，重复即 G2 失败，避免 Lua table 覆盖导致早期测试静默丢失。
- `*_helpers.lua` 不能定义可被该 lint 识别的任何 `test_<name>` entry，无论 RHS 是什么；常见 assignment 与 function-definition form 都会被识别，避免测试漏进 helper。
- 每个 helper 模块 `tests.<stem>` 必须被同包 tests 目录下至少一个其他 Lua 文件 `require("tests.<stem>")`；`require("tests.<x>")` 必须指向存在的 `tests/<x>.lua`，且测试文件和 helper 都不能 require 另一个 `*_test.lua` 模块。

G5 是全量测试的引擎真实输出覆盖检查：无参 `scripts/run.sh test` 会在全量测试结束后，从引擎 stdout 中锚定提取 `PASS <relfile>::<test_name>` 行，并要求每个 `packages/*/tests/*_test.lua` 文件至少产生一行被接受的 engine-format `PASS <relfile>::...`，否则说明该文件没有贡献任何引擎实际运行的测试，测试会失败并列出文件名。提取只接受整行精确匹配 `PASS <relfile>::test_<name>` 的行，其中 `<relfile>` 必须是本次实际扫描到的 `packages/*/tests/*_test.lua`，`<name>` 必须匹配 `test_[A-Za-z0-9_]+`；中间夹杂文本或 malformed `PASS ...` 不会计入。`scripts/run.sh test <pkg>` 是局部运行，会跳过 G5。

这些守卫是确定性、密闭、可移植的早期失败机制，但不替代引擎真实运行：它们不执行 router、不验证 reliable delivery 的 ack/retry/backoff/DLQ/lease/fencing，不做真实外部状态 reconciliation，也不触达 GitHub 或 `codex`。dup/helper 静态 lint 只覆盖常见 assignment 与 function-definition syntax；穷尽 Lua 枚举形态仍是 engine-owned audit 的 engine-PR backlog item。stdout 形态的 G5 检查也只是 best-effort：锚定和文件白名单可以缩窄伪造面，但测试代码仍可能打印一行完全匹配真实格式的 `PASS <relfile>::<test_name>`；完全可信的测试 inventory 需要引擎提供机器可区分的结构化结果。外部 CLI 与真实事件流仍由 `fkst-framework test`、`conformance`、`run`、`supervise` 和只读 dogfood 覆盖，G5 的引擎真实输出仍是 runnability authority。

engine-PR backlog：

- 引擎侧 Lua loader / sandbox 的 stray-global 与 unused-local audit。
- 引擎测试结果需要机器可区分的结构化输出：例如唯一 tag 的结果行、专用 channel，或 machine-readable inventory。当前 stdout-based G5 检查只能锚定真实格式并限制到已扫描测试文件，不能完全消除测试代码自行打印匹配 `PASS <relfile>::<test_name>` 的伪造风险。
- router / reliable-delivery hermetic harness：注入 source event，断言 route、ack、retry、backoff、DLQ、lease、fencing。
- deterministic `supervise --oneshot` / fixture runner，用于 package-level end-to-end reliable-delivery smoke。
- 真实 GitHub / `codex` JSON-shape reconciliation，作为 `workflow_dispatch` 或本地只读 dogfood；需要凭证，不进默认 CI。

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
- `FKST_GITHUB_WRITE=1` 是唯一写入姿态开关；未设置或不等于 `1` 时只 dry-run，不调用 mutate GitHub 的 `gh` 命令；设为 `1` 时 `github-devloop` 直接自治执行真实写入。
- `gh` traffic from `github-proxy` and `github-devloop` is tagged with the engine named rate pool `gh` (`burst=50`, `refill_per_hour=3250`). Real supervise runs fail closed unless `FKST_RATE_POOL_ROOT` is an absolute host-stable shared path, so package and website supervisors spend one GitHub budget instead of separate local budgets. 中文补充：这是集中令牌桶/共享速率池治理，不是 package 侧 sleep。
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

`packages/consensus/` 是通用、多角度的 flat 共识引擎，不绑定 GitHub 或 autochrono。它消费抽象 `proposal`，在一个 pipeline 内启动多个 peer-invisible 的 `codex exec` 角度；全体角度一致（全 approve/全 reject）时走 fast-path 产出 `consensus_reached`，否则启动第 4 个 meta-judge codex 读三角度输出，判 `reached:<framing>`（达成即决定 → `consensus_reached`）或 `converge:<narrowed_question>`（产出 bounded `consensus_converge`，带收窄问题 + 角度 digest，交消费侧以 round+1 收窄重发）。consensus 无状态、不持轮次、不盲循环、不拆分。

- 输入 schema 是 `consensus.proposal.v1`（可带 `round` / `convergence_question` / bounded `prior_round_digests`），输出 schema 是 `consensus.consensus_reached.v1` 或 `consensus.consensus_converge.v1`。`consensus_converge` 带 `schema`、`proposal_id`、`round`、`narrowed_question`、bounded `angle_digests`、`dedup_key`、`source_ref`，不带上轮 peer 全文（保持 peer-invisibility）。
- department 内只消费/产生裸名队列；被 composed 包引用时，对外表现为 `consensus.proposal`、`consensus.consensus_reached` 与 `consensus.consensus_converge`。
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

`packages/github-devloop/` 是组合 `github-proxy` + `consensus` 的 composed 包。它覆盖 issue design consensus、no-consensus 收敛与真停滞 reconcile、ready-to-implement：带 `fkst-dev:enabled` 且最新 GitHub 评论里没有本 proposal 的 `fkst:github-devloop:state:v1` marker 的 issue 被回源读取 body，映射成 `consensus.proposal`，并先写 `thinking` state marker；当 `consensus.consensus_reached` 返回 `approve` / `reject` 时，只有最新 state marker 仍是 `thinking` 才转到 `ready` / `blocked`，发一条带新 state marker 与 `fkst:github-devloop:result:v1` marker 的评论作为外部 durable 记录；`approve` 同时产生包内裸名 `devloop_ready` 事件（schema `github-devloop.ready.v1`）。

当 `consensus.consensus_converge`（带 `narrowed_question` + bounded 角度 digest）到达时，`departments/loop` 回源读取当前 issue 标题、版本和评论 marker：最新 state marker 仍是 `thinking` 时，从 trusted `fkst:github-devloop:converge-round:v1` marker 求当前 round = max(事件 round, marker round)，写本轮 converge-round marker（记 round + 问题/裁决 digest），并带 `narrowed_question` 以 round+1 重新 raise `consensus.proposal`，让下一轮角度只收窄到分歧点（保持 peer-invisibility，下轮只见收窄问题与 bounded digest，不见上轮 peer 全文；完整 issue 内容由 proposal 的 `content_fetch` 回源读取）；当 router 判 true-stall（round≥3 且最近连续三轮 question 与 verdicts digest 全相同）时不再重发，而是产生包内裸名 `devloop_reconcile` 事件（schema `github-devloop.reconcile.v1`）。`consensus` 不再有 `consensus_unresolved`、不盲循环、不拆分。

`departments/reconcile` 消费 `devloop_reconcile` / `devloop_review_reconcile`（按 payload schema 分发），是确定性判（**不跑 codex**）：回源确认最新 state marker 仍是 `thinking`（PR 侧 `reviewing`）且版本段匹配后，默认且当前唯一可达的 action 是 `drop`，写 `blocked` state marker 与 `fkst:github-devloop:reconcile:v1`（PR 侧 `review-reconcile:v1`）marker，语义是「放弃这个框架」——**不拆分子 proposal、不直接升级人、不在无共识时强行推进**。`re-design` / `re-cluster` 需要可信 directive fact，当前不可达。reconcile marker 的 version 段（`<base>/loop/<round>` 或 `<issue_version>/review-loop/<round>`，stage_rank `blocked`=800）使其压过 thinking/reviewing 并对重放幂等；最新 state marker 已是 `blocked` 或更靠后终态则跳过。

`departments/implement` 消费 `devloop_ready`，回源确认最新可信 state marker 仍是 `ready` 后，用这个 ready-CAS 门控一次实施尝试：调用 SDK `setup_worktree("devloop-" .. safe_issue_slug)` 创建隔离 git worktree，并用 `spawn_codex_sync({worktree=...})` 在该 worktree 内实施；ready-CAS gates the attempt，失败时写入 `impl-failed` state marker 与 failure marker；Codex 与 `git -C <worktree> status --porcelain` 成功且有变更时，写入当前版本、记录 worktree path 的 `implementing` state marker。Codex 非零退出或 status 为空会写入 `impl-failed` state marker 与 failure marker。

Phase 4 里，`open_pr` 在 `implementing` 且 `FKST_GITHUB_WRITE=1` 时，经 `github-proxy.github_pr_open_request` 打开 PR 并写 issue/PR linkage marker；dry-run 时只记录 would-open。`github-proxy` 写前会重导 implementing state / implementing fact / branch head / expected head，确认 devloop 侧派生事实仍匹配，而不读取人工授权 label。`observe_pr` 通过 PR 上可信 `fkst:github-devloop:pr-origin:v1` backpointer 把 issue 从 `pr-open` 推进到 `reviewing`，同时产生包内 `devloop_reviewing` 事件。Phase 5a/5b 里，`review_pr` 消费 `devloop_reviewing`，回源确认 issue canonical state 仍是同版本 `reviewing` 后，构造 source-agnostic `consensus.proposal`（`github-devloop/pr-review/.../<head_sha>`）：payload 只带短 brief、reviewed `head_sha`、`source_ref` 与 `content_fetch`，由 consensus codex 回源执行 `gh pr diff` 并读取 backing issue 全量内容；diff 和 issue 文本不进 payload。`review_result` 消费该 pr-review proposal 的 `consensus.consensus_reached`：它重新读取 PR trusted backpointer 和当前 head，要求当前 head 仍等于 review proposal 绑定的 `head_sha`，再以 issue state marker CAS 把 `reviewing` 推进到 `merge-ready`（`approve`）或 `fixing`（`reject`），写 issue state marker、`fkst:github-devloop:review-result:v1` marker、`merge-ready:v1` fact marker 与 set-exclusive label；`approve` 产生包内 `devloop_merge_ready`，`reject` 产生包内 `devloop_fixing`。

`fix` 消费 `devloop_fixing`，只在 issue canonical state 仍是 `fixing` 且 reject review marker、PR origin、PR head、deterministic branch 与 open same-repo PR 都回源匹配时运行 codex 修复。修复在同一 deterministic branch worktree 内提交；更新 PR 分支只由 `FKST_GITHUB_WRITE=1` 从 dry-run 切到真实写入，写前再次回源校验 issue/PR/head，使用非 force `git push origin <branch>`，推送后验证 PR head 等于新 commit，成功才写新的 `reviewing` state marker（version = `core.next_fix_version` 生成的 new-head fix-round canonical version）并产生新的 `devloop_reviewing` 进入 re-review。缺写开关只 dry-run，不推进；codex 无变更则进入 `review-meta`。

`review_loop` 消费 pr-review proposal 的 `consensus.consensus_converge`，复用同一收敛 doctrine：保留 PR 重导 + head 重校验 + reviewing version-CAS 守卫后，写 `fkst:github-devloop:review-converge-round:v1` marker（绑 review_proposal / issue_proposal / issue_version / head_sha / source_ref digest）并带 `narrowed_question` 以 round+1 重发同一 PR head 的 review proposal 收窄；true-stall 时产生包内 `devloop_review_reconcile` 交 `reconcile` drop 到 `blocked`。`review_meta` 不再由 review loop 预算触发，现仅由 `fix`（codex 无新 head 时）产生 `devloop_review_meta` 进入，只接受严格相邻的 `⟦FKST:ACTION⟧ fix|block` 与 `⟦FKST:REASON⟧ ...`，中和 issue/comment 文本，解析失败/歧义 fail-closed 到 `block`；`fix` 回到 `fixing` 并产生 `devloop_fixing`，`block` 推进 `blocked`。`review_meta` 无 `accept` 路径、不产 `merge-ready`：批准合并是 PR-diff review consensus（`review-result:v1 approve`）的唯一职责，删掉 `review_meta accept` 即移除了被自取 untrusted 内容伪造 marker spoof 到 `merge-ready` 的整条路径。

Phase 6 里，`merge` 消费 `devloop_merge_ready`，在写入时重新回源校验全部 gate：issue 最新可信 state marker 仍是同版本 `merge-ready` 或失败重试中的 `merging`、可信且 head-bound 的 `merge-ready:v1` comment-stream review-approval fact 与事件字段完全匹配、`review_proposal_id` 解析后仍指向同一 repo / PR / version 派生链 / reviewed `head_sha`、`FKST_GITHUB_WRITE=1`、可信 `review-result:v1 decision=\"approve\"` marker 与同一 `review_proposal_id`、`review_dedup_key`、issue proposal、`reviewed_head_sha` 和 version 绑定、PR 的当前 head 仍 open / same-repo / head branch 与 reviewed `head_sha` 未变、`gh pr view --json statusCheckRollup` 显示 CI green、`mergeable` / `mergeStateStatus` 可合并。`review_meta` 已无 `accept` 路径，不写 `merge-ready:v1`、不产 `merge-ready` 状态，因此不可能触发 merge；唯一能产生 `merge-ready` 与满足 merge 的 `review-result:v1 approve` backstop 的是 PR-diff review consensus。`github-devloop` merge 不使用 GitHub `reviewDecision` / `latestReviews` / `addPullRequestReview`，也不生成 merge-time codex。只有全部满足才先由本 bot 直接写可信 `merging:v1` marker，再执行普通 `gh pr merge --merge --match-head-commit`，不使用 admin override、不绕过 branch protection；随后写 `merged` state marker、`merged:v1` marker、set-exclusive `fkst-dev:merged` label，并执行 `gh issue close`。GitHub branch protection 的 required status checks 是真实无人值守 merge 的必需仓库运维前提：bot 账号不能有 bypass/admin override；Lua 的 `statusCheckRollup` 只是早期诊断和重试 backstop，真正不可绕过的 gate 是 GitHub 在 `gh pr merge` 时服务端强制的 branch protection。若重试时 PR 仍 open / same head / not merged，会重新推导全部 gate 并再次执行 `gh pr merge`；若重试时 PR 已是 `MERGED`，只要 `FKST_GITHUB_WRITE=1`、PR 仍匹配同 repo/branch/head，且存在匹配当前 PR/head 的本 bot `merging:v1` marker 或 canonical `merging` state marker，就会自愈 finalize。外部 merge 没有 bot `merging` fact，不会被 devloop 自动关闭 issue 或写 terminal marker。缺可信 `review-result:v1 approve`、缺可信 `merge-ready:v1` approval fact、缺写开关、CI pending 或 mergeability 未确定只 dry-run 或 retry 且不推进；CI red、明确不可合并或 PR head 在写前重导时前进会写 `merge-gate:v1` marker，回到 `fixing` 并产生 `devloop_fixing`；merge/close 命令失败则 error 交给可靠投递重试。

所有状态转移都以“最新可信 state marker + version CAS”为事实门：只信 `FKST_GITHUB_BOT_LOGIN` 对应 bot 作者的 marker，当前等于目标状态即幂等跳过，当前尚未到达前置状态则 error 触发可靠投递重试，incoming version 旧于当前 marker version 或当前已推进/分叉则跳过旧 replay。GitHub 是 eventually-consistent fact source，不是 strong-consistency KV；同 issue transition 共用同一个 `with_lock` key，marker 写入按 dedup 幂等，每次投递回源重导并自愈，读-CAS 到异步写 marker 的小 race window按最终一致性收敛。`fkst-dev:<state>` label 只是 set-exclusive、best-effort、自愈的 UI hint：每次转移都会请求加目标状态 label 并移除其他状态 label，但 correctness 不依赖 state label，`github-proxy` 不再做 state label 缺席预检。label/comment 写入都经 `github-proxy` 的 dry-run-by-default 出站队列；`FKST_GITHUB_WRITE` 是唯一姿态开关，未设置时只 dry-run，设为 `1` 时直接自治真实写入。Phase 6 的 `gh pr merge` / `gh issue close` 是最终危险写，由 `merge` department 直接执行并要求可信 head-bound `merge-ready:v1` review-approval fact、独立可信 `review-result:v1 approve`、CI/mergeability gate、head-bound `--match-head-commit` 全部满足。运行真实 supervisor 时应从 pinned 的 engine/package revision 启动，而不是 mutable dev HEAD；这样一次错误自动 merge 会影响未来仓库状态，但不会改变已经运行中的实例代码。残余风险需要诚实接受：bot 账号被攻破可伪造可信 marker；LLM 独立 review 仍是 bot 派生判断，不是客观证明；branch protection 是仓库运维配置，package 不能在 Lua 里查询、配置或强制；pinned runner 只能避免运行中实例自我变异，不能消除已经写入 GitHub 的后果；sshx 也不授权 commit/push/merge。`github-devloop/composed.deps` 声明它需要把 `github-proxy` 与 `consensus` 一起加载做组合 conformance。
