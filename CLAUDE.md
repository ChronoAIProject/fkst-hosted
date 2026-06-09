# CLAUDE.md

## 工作语言

源文件内部一律英文：`.lua`、`.sh`、`.py`、`.rs` 等里的注释、docstring、log/error 文本、模板字符串和标识符都保持英文，与 fkst-substrate 引擎、命令行工具和 LLM 语料一致。源文件之外的对外产物（对话回复、文档、issue/PR/comment、变更说明）用中文；代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。不要中英混杂凑句子。

## 这个仓库是什么

fkst-packages 是 fkst 的**包库**（"库 B"），承载跑在 **fkst-substrate** 引擎上的 Lua package。引擎本身在隔壁 `fkst-substrate` 仓；**本仓只写 Lua 行为层，不碰引擎 Rust**。

一个 package = `core.lua`（包内共享库）+ `departments/<dept>/main.lua`（消费/产生事件的处理器，暴露 `M.spec` 与 `pipeline(event)`）+ `raisers/<r>.lua`（cron/file_watch 触发器）+ `tests/*_test.lua`。包分两类：flat 平包必须自洽、可单根 conformance、0 外部 package namespace 引用；composed 包是一等包，负责组合/适配兄弟包，可引用 `<pkg>.<queue>`，用 `composed.deps` 声明组合 conformance 需要一起加载的兄弟包。当前 flat 包：`packages/github-proxy/`（GitHub issue/PR 入站同步 + 出站评论/label）和 `packages/consensus/`（消费抽象 `proposal`、一个 pipeline 内多角度 codex 共识 + 第 4 个 meta-judge codex 读三角度输出收窄，产出 `consensus_reached` / `consensus_converge`（带 `narrowed_question` + bounded 角度 digest）的通用 source-agnostic 共识引擎）。当前 composed 包：`packages/autochrono/`（消费自有 `issue` → 映射成 `consensus.proposal`，再消费 `consensus.consensus_reached` 产出自有 `reply`，组合 `consensus`）、`packages/github-autochrono/`（组合 `github-proxy` + `autochrono`）和 `packages/github-devloop/`（组合 `github-proxy` + `consensus`，用 GitHub 评论 `state:v1` marker 作为状态事实、`fkst-dev:<state>` label 作为自愈 UI hint，no-consensus 不盲循环也不拆分：`loop` 消费 `consensus_converge` 写 converge-round marker 并带 `narrowed_question` 重发 `proposal` 收窄收敛，router 判 true-stall（round≥3 且连续三轮 question+verdicts digest 不变）时 raise `devloop_reconcile` 交确定性 `reconcile` 部门 drop 到 `blocked`（语义「放弃这个框架」，无 codex、不 split、不升级人）；在 `devloop_ready` 上用隔离 worktree 进入 implementing，失败写入 `impl-failed` 终态 marker；PR 进入 `reviewing` 后复用 `consensus` 对 bounded PR diff 做 review decision，`approve` 推进 `merge-ready` 并产生 `devloop_merge_ready`、`reject` 推进 `fixing`，`fixing` 在 `FKST_GITHUB_WRITE=1` 时经写前重导、same-repo PR/head 校验与非 force push 回到新 head 的 `reviewing`；pr-review 的 `consensus_converge` 进入独立 review loop 同样写 review-converge-round marker 收窄收敛，true-stall 时 raise `devloop_review_reconcile` 交 `reconcile` drop 到 `blocked`；`review_meta`（`fix|accept|block` → `fixing|merge-ready|blocked`）不再由 review loop 预算触发，现仅由 `fix` 产出无新 head 时进入；`open_pr` 与 `merge` 没有人工 label gate 或模式开关，唯一姿态开关是 `FKST_GITHUB_WRITE`：未设置只 dry-run，设为 `1` 则直接自治真实写入；`merge` 必须有可信 `review-result:v1 decision="approve"` 与同一 review proposal/dedup/issue/head/version 绑定，`review_meta accept` 不足以 merge；`merge` 在 `merge-ready` 或失败重试中的 `merging` + `FKST_GITHUB_WRITE=1` 且 PR open/same-repo/head 未变、存在可信 head-bound `merge-ready:v1` comment-stream review-approval fact、CI green、mergeable 时执行普通 `gh pr merge --merge --match-head-commit`，写 `merging`/`merged` marker 并关闭 issue，缺 gate dry-run，CI 红或明确不可合并回 `fixing`；merge 不使用 GitHub `reviewDecision` / `latestReviews` / `addPullRequestReview`，不使用 admin override，真实全自动运行要求仓库 branch protection required status checks 服务端强制且 bot 不具备 bypass/admin override）。

## 引擎上下文（写包必须懂；权威见 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md`）

- **三级公司**：Company（supervisor + framework + composed graph）/ Department（`departments/<dept>/main.lua`）/ Person（一次 `codex exec`）。不能加层。
- **事件流** `source → fanout → route → spawn → RAISED`：raiser 静态声明 cron/file_watch；Department `M.spec` 静态声明 `consumes/produces/fanout/stall_window`。Department 收到的是 `Event{queue, payload, ts}`，**无生命周期 hook、无共享内存、无持久态**，同一 `pipeline` 跑两次是两次独立调用。
- **SDK surface（固定）**：`raise / spawn_codex_sync / spawn_codex / exec_sync / await_all / with_lock / once / cache_get / cache_set / git_log_count / git_log_grep / count_worktrees / setup_worktree / file / json.decode / log.{info,warn,error} / now`（+ test 模式 `fkst.test.{eq,is_true,is_nil,raises,run_department,mock_command,command_calls}`）。`json` 仅 `json.decode`。**包不直接碰 `<RT>`/文件系统当状态**——经原语。`once`/`cache_*`/`with_lock` 的 key 是经校验的**可读相对 path**（如 `github-proxy/issue/owner/repo/42`），不是 hex。测试中的外部 CLI 统一走 `fkst.test.mock_command` / `fkst.test.command_calls`，不生成 fake `gh` / `codex` 二进制；未 mock 的外部命令 fail-closed。
- **事实源 doctrine**：跨 pipeline 的真相只来自 git / 外部源（GitHub）/ 明确 host fact。GitHub 是 eventually-consistent authenticated fact source，不是 strong-consistency KV；读 GitHub marker 当事实时只信本 bot 作者（`FKST_GITHUB_BOT_LOGIN`，真写 `FKST_GITHUB_WRITE=1` 时未配置 fail-closed），用 state marker 的 `version` 做版本有序 CAS，version 总序是 `(updated_at ISO, loop round N, stage_rank)`，同 timestamp 下较大 `/loop/N` 胜过早期 loop，即使早期 marker 阶段更靠后。靠同 issue 统一 `with_lock`、幂等 marker 写入、可靠投递重导和自愈收敛。no-consensus 收敛轮次记在 converge-round / review-converge-round trusted-bot marker，true-stall reconcile 在锁内重导并按 reconcile / review-reconcile marker 幂等跳过已可见的同 round 结果、并 pin 当前 state 与版本段（thinking/reviewing 且 version 段匹配）才落 `blocked`；reconcile 是确定性判（drop→blocked），无 codex，因此不存在两个同 version codex 写出矛盾结果的窗口。包不在源码树或 `<RT>` 存"为活过崩溃"的业务状态；恢复靠 raiser 从源重导 + 下游按 `dedup_key` 幂等。源码树运行期只读。
- **可靠投递 / durable delivery（substrate dev 已合并）**：投递默认可靠，事件经 redb 持久 delivery（at-least-once-until-ack、lease+fencing、retry+backoff、DLQ）。对包作者：
  - **raise 到可靠下游的事件要带 `source_ref = {kind, ref}`**（稳定指针；消费者据此**回源 derive 当前真相**，不信可能过期的 payload；缺失会 fail-closed）。github-proxy 用 `{kind="external", ref="<repo>#<type>/<number>"}`（见 `core.entity_source_ref`）。
  - **【宪法·内容不入 payload】大体量内容（issue body / PR diff / 评论 / 代码 / 文件）绝不整体序列化进可靠投递 payload。** redb 可靠投递 payload 受 ~64KiB 静态上界约束；把内容塞进去 → 被迫机械截断（body 12000 / diff 8000 / digest 600 之类）→ 丢失全貌、codex 看不全、还反复跟 64KiB 死磕（dogfood 实证的反模式）。**内容传输是文件系统 / 网络的职责，不是投递管道的职责。** payload 只承载 `source_ref` 指针 + 小体量控制字段（schema / dedup_key / version / round / 短 digest）；需要内容的 codex / department **据 `source_ref` 从源自己 fetch 完整内容**——`gh issue view` 读全 issue + 全部评论、`gh pr diff` 读完整 diff、worktree / 文件系统读代码、网络读资源——拿全貌、无文字上限。这是「回源 derive 真相」doctrine 的硬化：截断快照本就违背它。现存的机械文字上限（`max_body_len` / `max_pr_diff_len` / `max_*_len` 等把内容塞 payload 再截断的设计）一律视为待迁移技术债；**新代码不得新增此类设计**，需要把内容给下游 codex 时，给 `source_ref` + 让它回源 fetch，而非塞进 payload。
  - `M.spec.ephemeral = {"queue"}` 把某 consumed queue 退化成内存 at-most-once；`M.spec.retry = {max_attempts, base, cap}` 调重试，`retry=false` = 失败不重试（仍可靠投递）。
  - **真实 `supervise` 运行需 `FKST_DURABLE_ROOT`**（redb 落点，**不是**可清的 `FKST_RUNTIME_ROOT` scratch）；有可靠订阅却缺它会启动 fail-closed。

## 包结构约定

- **包内共享库放 package-root**：`packages/<pkg>/core.lua`，department 内 `require("core")`。**只做包内共享**——不跨包 require、不建 `fkst/` 目录、不引包间版本管理。
- **flat 包 vs composed 包**：flat 包必须自有契约、自有裸名队列、0 外部 package namespace 引用，并通过单根 conformance；composed 包可以引用兄弟包 namespace 做组合/适配，但必须放 `composed.deps` 声明所组合的兄弟包，并经组合 conformance 验证。`composed.deps` 是测试组合的最小约定，不是版本/依赖解析 manifest，也不是部署配置；这是本仓为了让组合 glue 成为 CI 覆盖的一等包而接受的取舍。
- 事件带 `schema` 字段（如 `"github-proxy.v1"`）；幂等靠 `dedup_key`（+ 出站用评论里的 HTML marker 等外部 durable 源）。
- 出站写外部（如 `gh issue comment`）会改外部状态：默认 dry-run，真写只由 `FKST_GITHUB_WRITE=1` 表达。`github-devloop` 本质是直接自治系统，不保留历史兼容、双模式、人工 label gate 或 opt-in 写入开关；不可逆 merge 仍必须满足可信 marker、独立 PR diff `review-result:v1 approve`、head-bound、CI/mergeability、branch protection 与写前重导。

## 面向对象基本原则

- **单一职责原则**：一个类应该只有一个发生变化的原因。
- **开闭原则**：软件实体应该对扩展开放，对修改关闭。
- **里氏替换原则**：所有引用基类的地方必须能透明地使用其子类对象。
- **依赖倒置原则**：高层模块不应该依赖低层模块，二者都应该依赖其抽象；抽象不应该依赖细节，细节应该依赖抽象。
- **接口隔离原则**：客户端不应该依赖它不需要的接口；一个类对另一个类的依赖应该建立在最小的接口上。
- **迪米特法则**：一个对象应该对其他对象保持最少的了解。
- **合成复用原则**：尽量使用对象组合，而不是继承来达到复用的目的。

## 设计模式原则

- **模式服务当前问题**：只有当重复形状已经出现、边界已经稳定、测试能证明收益时才引入设计模式；不要为了命名完整而提前套 Factory、Strategy、Observer 等模板。
- **显式优先**：Lua package 中优先使用普通函数、table 和清晰数据流表达模式；避免隐藏控制流的全局注册表、自动发现、动态 monkey patch 和深层 metatable。
- **边界模式固定**：外部系统接入优先用 Adapter，把 `gh`、`codex exec`、文件和网络形态转成包内稳定结构；副作用边界集中，业务函数保持可单测。
- **分支策略清晰**：当同一流程因类型、来源或目标变化产生分支时，优先用 Strategy 形态的小函数表或显式 dispatch table；每个分支要有窄测试，不把条件散落在 `pipeline` 多处。
- **模板流程克制**：确有固定步骤、可变局部时才用 Template Method 形态的高阶函数；步骤顺序必须在代码中直观可读，不能让 hook 改变事件契约或投递语义。
- **组合包即 Facade**：composed package 是跨包组合的 Facade / Adapter 层，只做协议映射、队列 wiring 和最小编排；不要把兄弟包内部逻辑复制进 composed 包。
- **可删除性**：任何模式都要能被一个更直白的函数实现替换；如果删除模式后代码更短、更清楚、测试不变，优先删除模式。
- **门控即管线**：自动化系统里的"门控/决策"用一个 codex 判断管线 + event 流转开关表达，**不是人逐 event 加 label 授权**。人只控制哪些判断管线在跑（event 流转拓扑），不逐条介入：`auto 关 = 把 event 丢死信/丢掉`（没管线处理→不流转），`auto 开 = 一个管线处理它`（codex 判断决定流转并写 forge-guarded marker）。需要"可否/该不该自动处理"的判断时，新增一个保守的 codex 判断 dept（如 issue intake 判断哪些 issue 可自动开发），而不是留一个人工 label gate。可逆/危险运行姿态用 host 环境事实表达（如 `FKST_GITHUB_WRITE` 的 dry-run vs real），不在代码里留模式分叉。FKST 本就是全自动系统：默认就是 codex 判断 + 管线流转，不为"人来把关"保留人工授权门控。

## 构建 / 测试 / dogfood

- **引擎二进制**：本仓不含引擎。`cp env.example .env` 填 `BIN=<fkst-substrate>/target/debug/fkst-framework`。`scripts/run.sh` 按 `BIN` 覆盖 > `.env` > PATH > 同级 `../fkst-substrate` 解析；CI 中 `BIN` 不可执行会直接报错，且 CI 不自动 build。
- **标准测试**：`scripts/run.sh test [pkg]` 是本地和 CI 的单一入口：先跑一次 `"$BIN" --self-test`（脚本按需给临时 `FKST_RUNTIME_ROOT`）。flat 包跑 `"$BIN" conformance --project-root packages/<pkg> --package-root packages/<pkg>` 和 `"$BIN" test --project-root packages/<pkg> --package-root packages/<pkg>`；composed 包跳过单根 conformance，但仍跑 `"$BIN" test --project-root packages/<pkg> --package-root packages/<pkg>`。无参全包测试收尾会按所有 `composed.deps` 递归收集 composed 包及其依赖，以仓库根为 `--project-root` 跑一次组合 conformance；`scripts/run.sh test-composed` 可单独跑这一步。test 模式含 `*_test.lua` 单测 + `fkst.test.run_department` 集成测，**不经 router**，故 test 模式不强制 source_ref；`gh`、`codex exec` 等外部 CLI 用引擎 mock，未 mock fail-closed。
- **dogfood / 真跑一次部门**：`scripts/run.sh run <pkg> <dept> [event-json]` 一次性调用 `fkst-framework run`，解码 stdout 上的 `RAISED: <base64(JSON 数组)>` 并 dump `<RT>`。脚本用临时（或复用已设的）`FKST_RUNTIME_ROOT`，**绝不设置 `FKST_GITHUB_WRITE`**。
- **真实 supervise**：`scripts/run.sh supervise <pkg>` 是薄封装真实事件循环，创建临时 `FKST_RUNTIME_ROOT` 和独立临时 `FKST_DURABLE_ROOT`，默认 `--project-root packages/<pkg>`（可用 `FKST_PROJECT_ROOT` 覆盖），并显式传 `--package-root packages/<pkg>` 与 `--framework-bin "$BIN"`。前台运行，`Ctrl-C` 退出；不搭 host harness、不模拟事件、不注入 fake `gh`。
- **本地 build / freshness**：`test/run/supervise` 在解析 `$BIN` 后，若 `$BIN` 可溯源到 `<fkst-substrate>/target/debug/fkst-framework`，会先 `cargo build -p fkst-framework` 确保与该 checkout 当前工作树一致；不 `git pull`、CI 不自动 build、无法溯源仅 warn 跳过，`FKST_NO_AUTOBUILD=1` 可跳过。`scripts/run.sh build` 仍是显式 `git pull && cargo build` 的更新命令。
- **CI**：`.github/workflows/ci.yml` 从 `fkst-substrate@dev` 构建 fkst-framework，然后调用 `scripts/run.sh test`。改包后 push `dev`/`main` 触发。

## Git 提交/分支规范

- **语言**：提交信息、PR 标题/正文、分支说明属对外产物，用中文；分支名本身、代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。不要中英混杂凑句。
- **分支**：集成/默认分支是 `dev`；不直接向 `dev` 提交，一律从 `dev` 切分支并开 PR。分支名用 `<type>/<kebab-topic>`，`type` 只能是 `feat|fix|docs|chore|refactor|test`。合并后删除分支，不留长期僵尸分支。
- **提交**：一个 commit 是一个自洽逻辑改动，不混入无关改动或格式化噪声。subject 用一行中文祈使句概括做了什么，约 50 字以内，不堆叠多事；改动多于琐碎时，空行后写 body，说明为什么、影响和取舍，关键词/符号/错误分类保持可 grep。改契约就改完整，旧形态从当前态删除；不留 deprecated shim / `.old` / `_legacy`。
- **PR / 合并**：对 `dev` 开 PR；标题中文，正文含动机、改动、测试证据（命令 + 结果）。CI 绿才合；合并用 squash，保持 `dev` 线性、一个 feature 一条 commit，subject 末尾保留 `(#PR)`。AI 生成的 PR 正文/变更说明末尾保留 `⟦AI:FKST⟧`。

## 纪律（沿用 fkst-substrate）

- 源文件内部英文；对外中文。错误分类要窄（避免 `general error`）；日志/commit/event payload 可 grep。AI 生成的对外文本末尾保留 `⟦AI:FKST⟧`。
- 单个源代码文件不得超过 1000 行（范围含生产源码、测试源码、脚本源码，.lua/.sh/.py/.rs 等），硬上限、不设豁免；先删死码/重复代码，再按稳定职责拆成包内子模块或多个 `*_test.lua`；不得用无职责边界的碎片化、空转发文件或 compat/legacy/shim 壳凑行数。
- 不留 deprecated shim / compat layer / `.old` / `_legacy`；改契约就改完整，旧形态从当前态删除。文档描述当前态，历史留 git。
- **不要历史兼容性，不兼容历史遗留逻辑**。系统只有当前态一种形态：改行为就全量切换，不为向后兼容保留双模式、opt-in 开关、manual/legacy fallback 分支或旧路径并存。需要可关的运行姿态时，用 host 环境事实（如 `FKST_GITHUB_WRITE` 的 dry-run vs real）表达，而不是在代码里留"新逻辑 + 旧逻辑"的分叉。删就删干净，包括随之失效的常量、helper、测试与文档。
- **集成分支拓扑是 github-devloop 的运行姿态，不是可随手改的临时设置**：autonomous 改动先进**集成分支**（`FKST_DEVLOOP_INTEGRATION_BRANCH`）缓冲、再经 rollup PR 受控回 `dev`；`dev` 受保护，autonomous 改动**不直接合进 dev**。运行中**不得擅自切 topology（如 integration→单分支 dev）、不得擅自删/改远程分支**——这些是用户的架构决策，不是助手能定的。删任何远程分支前必须先查谁依赖它（in-flight PR 的 base、tracking 分支）；GitHub 删 base 分支会自动关闭其全部 open PR。
- **hotfix 就只修那个 bug，不顺手改架构/换运行方式/做破坏性操作**。dogfood/运行中遇到**设计层问题**（如 sync↔rollup ping-pong）按「遇问题提 issue」处理 + 停下确认，**绝不擅自换方案绕过**（尤其不能用"切到 dev 直合"绕过用户刻意设的缓冲/门控）。不可逆/破坏性远程操作（删分支、关 PR、force push、改默认分支）一律先确认，即使 `/goal` 等机制在催"继续"。
- **引擎 Rust 改动属 fkst-substrate 仓**，不在本仓做；本仓只写/改 Lua package + 测试 + 包文档。引擎需要的新能力（新 SDK 原语等）先在 fkst-substrate 提 PR。
- 跨文档定位：引擎事实以 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md` 为准；本仓 `README.md` 说明包约定与命令。

⟦AI:FKST⟧
