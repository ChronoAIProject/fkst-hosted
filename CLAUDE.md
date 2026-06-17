# CLAUDE.md

## 工作语言

源文件内部一律英文：`.lua`、`.sh`、`.py`、`.rs` 等里的注释、docstring、log/error 文本、模板字符串和标识符都保持英文，与 fkst-substrate 引擎、命令行工具和 LLM 语料一致。例外：明确作为本地化资源表保存的 outward text values 可以使用目标语言的 UTF-8 字面量；这些文本必须保持源码可读、可 grep，禁止用 hex/base64/byte-escape/`string.char` 等 decode helper 隐藏。源文件之外的对外产物（文档、issue/PR/comment、commit message、变更说明）**一律英文**：英文是唯一准绳文本，不附加中文补注/restatement。代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。对话回复跟随用户语言。不要中英混杂凑句子。存量中文文档（含本文件）可保留中文，新增规范性文本英文优先。

## 这个仓库是什么

fkst-packages 是 fkst 的**包库**（"库 B"），承载跑在 **fkst-substrate** 引擎上的 Lua package。引擎本身在隔壁 `fkst-substrate` 仓；**本仓只写 Lua 行为层，不碰引擎 Rust**。

一个 package = `core.lua`（包内共享库）+ `departments/<dept>/main.lua`（department 入口处理器，暴露 `M.spec` 与 `pipeline(event)`；department **不限于单文件**——职责变大时可在同目录拆出 department-local 子模块，经 `require("departments.<dept>.<mod>")` 引入，`main.lua` 只作入口，如 `autochrono` 的 `propose/mapping.lua`）+ `raisers/<r>.lua`（cron/file_watch 触发器）+ `tests/*_test.lua`。包分两类：flat 平包必须自洽、可单根 conformance、0 外部 package namespace 引用；composed 包是一等包，负责组合/适配兄弟包，可引用 `<pkg>.<queue>`，用 `composed.deps` 声明组合 conformance 需要一起加载的兄弟包。当前 flat 包：`packages/github-proxy/`（GitHub issue/PR 入站同步 + 出站评论/label）和 `packages/consensus/`（消费抽象 `proposal`、一个 pipeline 内多角度 codex 共识 + 第 4 个 meta-judge codex 读三角度输出收窄，产出 `consensus_reached` / `consensus_converge`（带 `narrowed_question` + bounded 角度 digest）的通用 source-agnostic 共识引擎）。当前 composed 包：`packages/autochrono/`（消费自有 `issue` → 映射成 `consensus.proposal`，再消费 `consensus.consensus_reached` 产出自有 `reply`，组合 `consensus`）、`packages/github-autochrono/`（组合 `github-proxy` + `autochrono`）和 `packages/github-devloop/`（组合 `github-proxy` + `consensus`，用 GitHub 评论 `state:v1` marker 作为状态事实、`fkst-dev:<state>` label 作为自愈 UI hint，no-consensus 不盲循环也不拆分：`loop` 消费 `consensus_converge` 写 converge-round marker 并带 `narrowed_question` 重发 `proposal` 收窄收敛，router 判 true-stall（round≥3 且连续三轮 question+verdicts digest 不变）时 raise `devloop_reconcile` 交确定性 `reconcile` 部门 drop 到 `blocked`（语义「放弃这个框架」，无 codex、不 split、不升级人）；`ready → implementing` 受 issue 依赖 gate 约束：`core.dependency_gate` 据 GitHub 原生 `issue.blockedBy` 重导依赖、按各 blocker 的 trusted `merged` 判完成，未全满足时 hold 在 `ready`（写 `dependency-wait:v1`/`dependency-cycle:v1` marker + `fkst-dev:blocked-on-dependency` 辅助 label，不新增状态），blocker merge 后下一轮 poll 自动级联放行；环/缺失/跨 repo/blockedBy 截断/gh 失败一律 fail-closed hold（只 satisfied 才放行），gate 同时落在 `consensus_result`/`observe_issue`/`implement` 三处；在 `devloop_ready` 上用隔离 worktree 进入 implementing，失败写入 `impl-failed` 终态 marker；PR 进入 `reviewing` 后复用 `consensus`，通过 `source_ref` / `content_fetch` 回源读取完整 PR diff 与 backing issue 内容做 review decision，`approve` 推进 `merge-ready` 并产生 `devloop_merge_ready`、`reject` 推进 `fixing`，`fixing` 在 `FKST_GITHUB_WRITE=1` 时经写前重导、same-repo PR/head 校验与非 force push 回到新 head 的 `reviewing`；pr-review 的 `consensus_converge` 进入独立 review loop 同样写 review-converge-round marker 收窄收敛，true-stall 时 raise `devloop_review_reconcile` 交 `reconcile` drop 到 `blocked`；`review_meta`（`fix|block` → `fixing|blocked`，无 `accept` 路径、解析失败/歧义 fail-closed 到 `block`、不产 `merge-ready`）不再由 review loop 预算触发，现仅由 `fix` 产出无新 head 时进入；`open_pr` 与 `merge` 没有人工 label gate 或模式开关，唯一姿态开关是 `FKST_GITHUB_WRITE`：未设置只 dry-run，设为 `1` 则直接自治真实写入；`merge` 必须有可信 `review-result:v1 decision="approve"` 与同一 review proposal/dedup/issue/head/version 绑定（唯一 `merge-ready` 与 merge 授权权威是 PR-diff review consensus，`review_meta` 不参与 merge 授权）；`merge` 在 `merge-ready` 或失败重试中的 `merging` + `FKST_GITHUB_WRITE=1` 且 PR open/same-repo/head 未变、存在可信 head-bound `merge-ready:v1` comment-stream review-approval fact、CI green、mergeable 时执行普通 `gh pr merge --merge --match-head-commit`，写 `merging`/`merged` marker 并关闭 issue，缺 gate dry-run，CI 红或明确不可合并回 `fixing`；merge 不使用 GitHub `reviewDecision` / `latestReviews` / `addPullRequestReview`，不使用 admin override，真实全自动运行要求仓库 branch protection required status checks 服务端强制且 bot 不具备 bypass/admin override）。

## 旁路推理通道：nyxid oracle (ChatGPT Pro)

`nyxid oracle` 把推理任务路由到浏览器端 ChatGPT Pro，是 codex / Claude 之外的**旁路推理通道**。用途：复杂任务迟迟不出结果、需要新思路或独立复核时，让 ChatGPT Pro 跑一段独立推理 / 第二意见，再回到本管线决策。子命令、参数、输出字段以 `nyxid oracle --help` / `nyxid oracle ask --help` 为准（已验证子命令：`ask` / `result` / `cancel` / `status` / `attach` / `extract` / `pool` / `sessions` / `session` / `close-session`）。本节只记非显然用法，**不缓存配额/并发数字**：

- **两步异步（省 token，不轮询）**：`nyxid oracle ask company-chatgpt-pro "<问题>" --no-wait --output json` 拿 `task_id`（status=queued）；再 `nyxid oracle result <task_id> --output json` 取结果。status 走 `queued → dispatched(phase=sent) → completed`，`completed` 时 `response` 字段即答案（附 `chatgpt_url`）。单次 `result` 即可，未完成返回中间 status——**不要 busy-loop 轮询**。省 `--no-wait` 则 `ask` 同步阻塞最多 `--wait` 秒。
- **pool 是 org-visibility**：`company-chatgpt-pro` 限该 pool 所属 org 成员；非成员 `ask` 返回 403（`error_code 1002 forbidden`），`status` 返回 404。先 `nyxid org join <邀请码>` 加入该 org，再 `nyxid oracle pool list` 应能看到该 pool（看不到该 pool 即说明当前账号未加入对应 org）。
- 长 prompt 用 `--file -` 从 stdin 喂，附件 `--pdf`，多轮 `--new-conversation` / `--conversation <id>`；配额与并发（per-user inflight、worker tab 数）以 `nyxid oracle pool list` / `nyxid oracle status <pool>` 实时读出，不在此缓存。
- **本仓 sshx 标配并行 oracle（standing augmentation）**：在本仓做 sshx out-of-band 设计/修复（thinking triplet → meta-judge → implementation → review triplet → fix-or-done）时，**除 3 个 peer-invisible codex worker 外，同时把同一 GoalArtifact/diff 喂给 ChatGPT Pro oracle 作并行第 4 视角**（跨模型族独立推理/复核），与 codex triplet 同 round 派发、各自隔离。oracle 是 codex consensus 之外的**跨模型族交叉验证**，不替代 sshx 的 worker 契约（completion/verdict 仍由 codex envelope 路由,oracle 是 advisory 交叉信号）：thinking 阶段 oracle 与 triplet 并行找根因与设计洞;review/fix 阶段 oracle 复核实现的正确性与遗漏。实证价值（2026-06-17）:#873 false-terminal 的无界-liveness 洞由 codex review triplet 与 ChatGPT Pro oracle **独立收敛**到同一根因;「伞机械生命周期」doctrine 由「sshx 4 视角(3 codex + ChatGPT Pro 跨模型族)」近一致收敛得出。oracle 不可用(403/404/配额耗尽)时 sshx 仍按 codex triplet 正常进行,oracle 缺席只损失交叉验证、不阻断。

## 引擎上下文（写包必须懂；权威完整的 engine↔package 契约见 fkst-substrate 的 `docs/package-repo-contract.md`，引擎实现细节见其 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md`）

- **三级公司**：Company（supervisor + framework + composed graph）/ Department（`departments/<dept>/main.lua`）/ Person（一次 `codex exec`）。不能加层。
- **事件流** `source → fanout → route → spawn → RAISED`：raiser 静态声明 cron/file_watch；Department `M.spec` 静态声明 `consumes/produces/fanout/stall_window`。Department 收到的是 `Event{queue, payload, ts}`，**无生命周期 hook、无共享内存、无持久态**，同一 `pipeline` 跑两次是两次独立调用。
- **SDK surface（固定；权威完整列表与签名见契约文档）**：经原语访问 `raise / spawn_codex_sync / spawn_codex`（`opts.timeout` 控 codex 整体超时，默认 3600s）`/ exec_sync / exec_argv / await_all / with_lock / once / cache_* / git_log_* / count_worktrees / setup_worktree / file / json.decode`（仅 decode）`/ log / now` 等，外加 test 模式 `fkst.test.*`（`mock_command` / `command_calls` / `run_department` 等）。`exec_sync` 是 genuine-shell primitive；`gh`/`git` egress 只经 `std.github`/`std.git` 构造 argv 并调用 `exec_argv`。**包不直接碰 `<RT>`/文件系统当状态**——经原语。`once`/`cache_*`/`with_lock` 的 key 是经校验的**可读相对 path**（如 `github-proxy/issue/owner/repo/42`），不是 hex。使用 ports 的业务代码访问 `gh`/`git` 时，不按 command string mock：访问经注入的 `std.github`/`std.git` handles，测试用 `std.testing.run_fake` + in-process `std.github_fake`/`std.git_fake`；`fkst.test.mock_command` / `fkst.test.command_calls` 仍用于其他外部 CLI（如 `codex`）以及 adapter-contract tests（command spelling 本身是被测对象时）。不生成 fake `gh` / `git` / `codex` 二进制；未 mock 的外部命令 fail-closed。
- **事实源 doctrine**：跨 pipeline 的真相只来自 git / 外部源（GitHub）/ 明确 host fact。GitHub 是 eventually-consistent authenticated fact source，不是 strong-consistency KV；读 GitHub marker 当事实时只信本 bot 作者（`FKST_GITHUB_BOT_LOGIN`，真写 `FKST_GITHUB_WRITE=1` 时未配置 fail-closed），用 state marker 的 `version` 做版本有序 CAS，version 总序是 `(updated_at ISO, loop round N, stage_rank)`，同 timestamp 下较大 `/loop/N` 胜过早期 loop，即使早期 marker 阶段更靠后。靠同 issue 统一 `with_lock`、幂等 marker 写入、可靠投递重导和自愈收敛。no-consensus 收敛轮次记在 converge-round / review-converge-round trusted-bot marker，true-stall reconcile 在锁内重导并按 reconcile / review-reconcile marker 幂等跳过已可见的同 round 结果、并 pin 当前 state 与版本段（thinking/reviewing 且 version 段匹配）才落 `blocked`；reconcile 是确定性判（drop→blocked），无 codex，因此不存在两个同 version codex 写出矛盾结果的窗口。包不在源码树或 `<RT>` 存"为活过崩溃"的业务状态；恢复靠 raiser 从源重导 + 下游按 `dedup_key` 幂等。源码树运行期只读。
- **Assignee claim doctrine**：`github-devloop` uses GitHub issue assignees as an optimistic lease and UI surface for multi-instance isolation. The protocol is current-assignees-only: an unmanaged unassigned issue may be assigned to `FKST_GITHUB_BOT_LOGIN`, then re-read before proceeding; any non-self assignee means skip, and every external write re-verifies that the same self-only claim is still held. Losing the claim is stop-on-discovery, while marker trust, version CAS, dedup, review gates, and merge gates remain authoritative. Timeout release is self-only: after a fresh assignee read, the package may remove only its own configured bot login, never a human or non-self assignee; dry-run posture logs the would-release without mutating GitHub.
- **可靠投递 / durable delivery（substrate dev 已合并）**：投递默认可靠，事件经 redb 持久 delivery（at-least-once-until-ack、lease+fencing、retry+backoff、DLQ）。对包作者：
  - **raise 到可靠下游的事件要带 `source_ref = {kind, ref}`**（稳定指针；消费者据此**回源 derive 当前真相**，不信可能过期的 payload；缺失会 fail-closed）。github-proxy 用 `{kind="external", ref="<repo>#<type>/<number>"}`（见 `core.entity_source_ref`）。
  - **【宪法·内容不入 payload】大体量内容（issue body / PR diff / 评论 / 代码 / 文件）绝不整体序列化进可靠投递 payload。** redb 可靠投递 payload 受 ~64KiB 静态上界约束；把内容塞进去 → 被迫机械截断（body 12000 / diff 8000 / digest 600 之类）→ 丢失全貌、codex 看不全、还反复跟 64KiB 死磕（dogfood 实证的反模式）。**内容传输是文件系统 / 网络的职责，不是投递管道的职责。** payload 只承载 `source_ref` 指针 + 小体量控制字段（schema / dedup_key / version / round / 短 digest）；需要内容的 codex / department **据 `source_ref` 从源自己 fetch 完整内容**——`gh issue view` 读全 issue + 全部评论、`gh pr diff` 读完整 diff、worktree / 文件系统读代码、网络读资源——拿全貌、无文字上限。这是「回源 derive 真相」doctrine 的硬化：截断快照本就违背它。历史上的机械文字上限（把内容塞 payload/codex prompt 再截断的 `max_*_len` 设计）一律视为待迁移技术债；**新代码不得新增此类设计**，需要把内容给下游 codex 时，给 `source_ref` + 让它回源 fetch，而非塞进 payload。
  - `M.spec.ephemeral = {"queue"}` 把某 consumed queue 退化成内存 at-most-once；`M.spec.retry = {max_attempts, base, cap}` 调重试，`retry=false` = 失败不重试（仍可靠投递）。
  - **真实 `supervise` 运行需 `FKST_DURABLE_ROOT`**（redb 落点，**不是**可清的 `FKST_RUNTIME_ROOT` scratch）；有可靠订阅却缺它会启动 fail-closed。

## 包结构约定

- **包内共享库放 package-root**：`packages/<pkg>/core.lua`，department 内 `require("core")`。**跨包共享放 repo-root `std/`**（单向、分层：经 `packages/<pkg>/std -> ../../std` 符号链接引入，`require("std.<m>")`；引擎对 Lua module 是 owner-scoped 硬隔离、无共享根原语，故 symlink 是当前唯一零引擎解，未来由引擎 `--lib-root` 共享根原语取代、symlink 删除）。`std` 当前分 Tier S（substrate 契约：`std.saga`、`std.saga_conformance`、`std.oracle`、version-CAS / `source_ref` helpers——可升 substrate）与 Tier R（本仓领域：`std.github`、`std.git`、`std.ports`、`std.testing`、`std.github_fake`、`std.git_fake`、`std.strings`、`std.github_debug_stamp`，永久留本仓）。绑定规则：new and migrated (non-allowlisted) `gh`/`git` command construction、shell quoting、execution、stdout parsing 必须在 `std.github`/`std.git` adapter 后面；package 经 `make_department(ports)` 接收注入 ports，new business code 不得构造 raw `gh`/`git` command heads；仍在 raw 构造 `gh`/`git` 的 existing call sites 是 `migration/gh-git-adapter.allowlist` 跟踪的 migration debt，G-ADAPTER ratchet 会持续收窄到这条规则。production port wiring 由 `std.ports.install` / `std.ports.production_handles` 集中负责，不按 department 复制。完整 ports/adapters rationale 与 surface taxonomy 见 `docs/superpowers/specs/2026-06-15-ports-adapters-design.md`。纪律:**禁 peer 跨包 require（A→B 内部，`check_repo.py` G9 强制）；只允许唯一 blessed 共享库根（all→std）**。`std` 不是包间版本管理 / manifest / 依赖解析。
- **按稳定职责拆文件，绝不为凑行数把多职责挤进单文件**：department 不必只有一个 `main.lua`——逻辑变大时按职责边界拆出 department-local 子模块（同目录，`require("departments.<dept>.<mod>")`，如 `autochrono` 的 `mapping.lua`），跨 department 复用的才上提到 package-root `core.lua`。`raisers/`、`tests/*_test.lua`、`core.lua` 等其他文件同理：满足 1000 行上限的唯一正解是**按职责拆成多个有边界的文件**，不是把逻辑硬塞进一个文件来「遵守」上限，也不是无职责边界地碎片化。
- **flat 包 vs composed 包**：flat 包必须自有契约、自有裸名队列、0 外部 package namespace 引用，并通过单根 conformance；composed 包可以引用兄弟包 namespace 做组合/适配，但必须放 `composed.deps` 声明所组合的兄弟包，并经组合 conformance 验证。`composed.deps` 是测试组合的最小约定，不是版本/依赖解析 manifest，也不是部署配置；这是本仓为了让组合 glue 成为 CI 覆盖的一等包而接受的取舍。
- 事件带 `schema` 字段（如 `"github-proxy.v1"`）；幂等靠 `dedup_key`（+ 出站用评论里的 HTML marker 等外部 durable 源）。
- 出站写外部（如 `gh issue comment`）会改外部状态：默认 dry-run，真写只由 `FKST_GITHUB_WRITE=1` 表达。`github-devloop` 本质是直接自治系统，不保留历史兼容、双模式、人工 label gate 或 opt-in 写入开关；不可逆 merge 仍必须满足可信 marker、独立 PR diff `review-result:v1 approve`、head-bound、CI/mergeability、branch protection 与写前重导。

## No Permission-Based Control / 禁止用文件权限做控制

Never use file or directory permissions as a control, guard, isolation, or read-only mechanism anywhere in this system. Production source must not add `chmod`, restrictive mode literals such as `0555` / `0444` / `0500` / `0400`, read-only directories, or any equivalent permission-removal scheme to enforce behavior. The only allowed permission operation is making a test fixture or probe executable, such as `chmod +x` in test code; that is fixture setup, not control-by-permission.

Directory permissions are fragile: a read-only parent prevents `git worktree add` from creating the leaf, breaks `rm -rf` cleanup, varies by OS/filesystem, and can fail silently enough to look like unrelated liveness drift. They are also redundant: runtime read-only and source immutability are enforced by process isolation, including codex `--sandbox read-only`, worktree isolation, and the engine's runtime-only-read source handling. The authority for control is isolation plus durable marker/CAS/saga facts, never file modes.

Incident of record (2026-06-17): `mkdir -p X && chmod 0555 X` on a worktree parent broke `sync_scan`'s `git worktree add`, stalled forward sync across a week's dev advance, left running code stale, and allowed recurrence of an already-fixed false-terminal class. ⟦AI:FKST⟧

## 面向对象基本原则

- **单一职责原则**：一个类应该只有一个发生变化的原因。
- **开闭原则**：软件实体应该对扩展开放，对修改关闭。
- **里氏替换原则**：所有引用基类的地方必须能透明地使用其子类对象。
- **依赖倒置原则**：高层模块不应该依赖低层模块，二者都应该依赖其抽象；抽象不应该依赖细节，细节应该依赖抽象。
- **接口隔离原则**：客户端不应该依赖它不需要的接口；一个类对另一个类的依赖应该建立在最小的接口上。
- **迪米特法则**：一个对象应该对其他对象保持最少的了解。
- **合成复用原则**：尽量使用对象组合，而不是继承来达到复用的目的。

## 核心循环：不分析原因，watchdog 心跳盲重投 + 乐观锁 + codex 兜底（简单优先）

系统**不追求「用程序完美枚举处理每一种失败」**。程序保持笨、健壮、确定；智能长尾交给 codex。**「不分析原因」是铁律，但触发重投的「超时」绝不能是裸 wall-clock**——裸定时器会在健康的长跑异步 receiver（implement codex ~2h / review consensus / CI 等待）**还在干活**时就开火，把健康工作当 strand 终结，反向重造 #762 要修的病（false-terminal，不是 frozen；实证 #762 8 轮 review 逐层逼出）。正解是 **watchdog timer 模式**（嵌入式经典 harness）：被监督的 receiver 周期性「踢狗」（写心跳 marker），watchdog 只在**狗没被踢**（心跳超预算变陈）时才动作。**关键：踢狗检测不是根因分析**——它是一个通用 liveness 探针（receiver 还在不在动？），**不问「为什么慢」**，所以「不分析原因、程序保持笨」原封不动；我们没加任何 per-case 分支，只加了**一个通用 liveness 信号**。恢复路径只有三条手段，按此顺序：

1. **watchdog 心跳盲重投（有界）**：每个非终止态声明它的 watchdog，二选一——(a) **budget-bounded**：无长跑 receiver、或工时上界已知时，预算 ≥ receiver 最大健康工时，超预算即真卡（pr-open / fixing / merge-ready 390m CI SLA 等）；(b) **heartbeat-deferred**：有长跑 receiver 时，receiver 周期写心跳 marker（**既有 bot marker 即心跳，不是新真相源、不动引擎**），心跳在预算内就 **defer**（让它干活），心跳变陈才动（implementing / thinking / reviewing）。两种都**不分析为何卡、不写 per-case 根因分支**；触发后的**动作仍是盲的**——盲目重投（重发驱动事件、version 单调 +1）。重投有界（sweep 自有 durable attempt 计数，从稳定血统派生、脱离 receiver 能否消费 redrive）；耗尽进入第 3 条。**一句话：重投动作是盲的（零分析），watchdog 心跳只决定「何时」投——这就把「盲」和「别误杀健康长活」调和了。**
2. **并发乐观锁**：一切并发用 version 全序 + CAS 兜住（乐观并发），**不写专门的并发协调**。陈旧重投被新版本盖过即可，func1 无需感知并发。
3. **搞不定 → catch → 结构化日志 → codex 兜底**：盲重投耗尽、或确定性路径明确处理不了的，`try-catch` 住、落**丰富可 grep 的结构化事实**（`error_class`/`fingerprint`/`source_ref`/WHY/`terminal`），写一个确定性终态（如 `blocked`-with-WHY），**交 codex 作智能兜底**——codex 只读消费这些事实、经 review 门（issue→PR→review→merge）起草修复或重立项。

心法：`func1: event→effects`（快、确定、可重放、**watchdog 心跳盲重投**、不枚举 case）；`codex: facts→受控产出`（慢、只读输入、过门生效）。**不要为追求「程序完美」去枚举每个失败形态写确定性分支——枚举不完，且每个分支都是新 bug 面（实证：想用程序把「终止」判得完美的精确匹配 reconcile 反而造出 livelock）。简单 watchdog 心跳盲重投 + 乐观锁兜住常态，长尾一律 codex 兜底。** watchdog 模式由 conformance 机械强制：每个非终止态必须声明 budget-bounded 或 heartbeat-deferred（heartbeat 行的 producer / surface / version-form 经单一真相源 helper 绑定 resolver），**新态不正确声明 receiver-liveness 就 conformance 失败**——把「只有 adversarial review 抓得到的 liveness bug」变成机械不变式（实证 #762：8 轮 review 每轮 tests 全绿却抓出更深的 liveness bug，正因 liveness-blind 正确性 CI 抓不到，才必须做成 conformance 契约）。下面的三级模型 / saga 化 / 活性契约都是这条的机械实现——用来让「简单」可被机械强制，不是要你手写每个 case 的确定性恢复。

## 这套自愈循环的成熟名字（prior-art 合成，harness-first）

上面的核心循环不是自创范式，是四套成熟工程理论的合成。按 harness-first，把名字钉清楚——新代码据此自检「我套用了哪条成熟实践、在哪偏离、为什么」：

1. **Durable / workflow state machine**（Temporal·Cadence 的 durable execution；Harel statecharts；table-driven FSM）：每个**生命周期状态**是 restart 表里的一行；`core/restart` 的 `restart_transition_table` 就是这张表。
2. **Saga pattern**（Garcia-Molina & Salem, 1987）：每次状态转移是一个有界、可补偿、保证终止的 saga step（强制 saga 化 #375）。
3. **Crash-only software · Recovery-Oriented Computing**（Candea & Fox）+ **supervisor tree ·「let it crash」**（Erlang/OTP, Joe Armstrong）：**不枚举失败形态**；把一切当 crash，靠**有界重启（OTP 的 max restart intensity）**恢复；**重启预算耗尽就向上逃逸到更聪明、更慢的 supervisor**。本系统最顶层的 supervisor 就是 **codex**（facts→issue，过 issue→PR→review→merge 门）。所谓「概率分析」就是这条：有界重试 + 逃逸，而**预算的取值即编码了失败概率阈值**——「重投/等了 N 还不愈，就判定它不是瞬态、是结构性长尾，逃逸给 codex」。预算是**设计期常量，不是运行期概率估计器**（后者会引入第二真相源）。
4. **Totality ·「make illegal states unrepresentable」**（type-driven design, Yaron Minsky）：conformance 强制**每个非终止态在表里都有完整一行**（budget + watchdog 模式 + 保证终止 + WHY）；缺一即 CI 失败。这把「简单」从约定变成机械不变式（#762）。

**一句话：本系统 = 一个 crash-only、durable、分层受监督的状态机；恢复是有界的 watchdog 心跳盲重投；最顶层的 supervisor 是 LLM。** 这正是为什么「状态转移 + saga + 概率分析（有界重试编码概率）+ 长尾 codex 兜底 + harness 强制全状态进表」让系统**简单明了**：这五条不是五个独立机制，是同一套成熟架构的五个面，合起来**只剩一种形状——填表的一行**。新增状态 = 填一行（声明 budget / watchdog / 终止 / WHY），不发明新控制流；N 个 per-case 确定性分支塌成「1 个有界盲重投 + 1 个 codex 兜底」；conformance 让这种简单**无法腐烂**。

**边界（防过度统一）：这张表治的是「生命周期状态」（marker-as-fact 状态机），不吞掉整个系统。** 事件路由（fanout/dispatch）、内容回源（source_ref→fetch）、ports/adapters egress、consensus 编排是**正交纪律**，各有各的成熟范式，不塞进这张表——硬塞违背「模式服务当前问题」。表统一 lifecycle，ports 治 egress，saga 治持久，codex 治长尾。

## 随时可重启 supervise（crash-only restart contract）

**部署即重启、随时可重启：`supervise` 必须能在任何时刻被 SIGKILL + 重启而不丢工作、不造成永久停滞。** 这是 crash-only software（Candea & Fox，见上一节）的硬契约，不是「尽量」。系统不做 drain / 优雅关停 / 在途排空；恢复靠两条既有机制：① **durable 投递**（redb at-least-once + lease/fencing + retry）让在途事件重启后续投；② **从 marker / git / 外部源回源 re-derive**（真相不在内存态）让任何中间态被重新推导、重驱。被重启杀掉的在途 codex（implement / fix / review）由其状态的 live-defer 心跳变陈后**重新 spawn**——所有工作幂等、可重入。重启因此是**无害的常规运营动作**（部署新代码、清运行态、换 BIN），随时可做，不需攒批次、不需等"安全窗口"。

**铁律：重启永不作为问题的解释。** 看到重启后某 strand 没进展时，**默认归因不是「重启 churn 掉了它」**——这是违背本契约的偷懒归因，会掩盖真缺陷（活性盲区）。crash-only 下重启理应被 durable + re-derive 吸收；若重启**确实**导致永久丢失/停滞，那必然是一个**活性契约缺陷**（durable 没续投、re-derive 没重导、或「心跳变陈 → re-spawn」链断了），要 root-cause + 提 issue，绝不用「重启影响了它」搪塞，也绝不为「避免 churn」去不重启 / 攒批次（那让进程长跑陈旧代码，反害——见 dogfood「立即重启别攒批次」）。运营随时重启；把工作活下来是**系统的责任**，不是运营的小心翼翼。实证（2026-06-17）：误把一个 fixing-loop 停滞甩锅给「我反复 restart churn 掉 fix codex」，实查发现重启后 fix codex 已被正常 re-spawn（crash-only 生效），真信号是另一处 marker-visibility version-desync——偷懒归因差点掩盖真缺陷。

## 错误处理三级模型（codex-as-catch）

任何流程 `A → func1 → B` 的失败处理分三级；**catch 的产出是「立项」而非「当场修」**（prior art：OTP 监督树要求快路径 supervisor 简单确定；AIOps 异常→工单；LLM 自愈模式的已知失败形态是不确定性与副作用越界）：

- **L1 确定性热路径**（毫秒-秒，引擎职责，**禁 codex**）：fail-closed、retry+backoff、lease/fencing、DLQ；每个失败落结构化错误事实（`error_class`、`fingerprint`、`source_ref`、`attempt`、`terminal`），重放必须确定。
- **L2 修复管线**（分钟-小时，包职责）：triage codex **只读**消费失败事实（dead_letter 事件、错误日志），按 fingerprint+时间窗去重后起草 issue（intent-before-create 防重）；修复一律经 issue→PR→review→merge——这是 codex「解决」错误的唯一合法形态。
- **L3 周期巡检**（小时级，包职责）：log-patrol codex 聚合跨切面/低频异常与停滞嫌疑，同样只产出去重 issue，绝不直接改运行态。

禁令：热路径不得 spawn codex；任何 catch 不得吞原始错误、不得改运行源码树、不得绕过 PR 门控、不得做 reconcile/CAS 级决策。「func1 与 codex 都是函数」的准确含义——`func1: event→effects`（快、确定、可重放）；`codex: facts→issue`（慢、只读输入、受控输出）。

## 活性 ⟂ 安全双检测（错误网抓不到「该发生而没发生」）

错误处理三级模型是**安全（safety）**侧——它抓「发生了坏事」：失败产生结构化错误事实（throw → fail-closed → retry → DLQ → L2 triage 消费）。但它对**活性（liveness）**违例**结构性失明**：「该发生的好事没发生」——一个本该 raise 的事件从未 raise、一个本该跑的 scan 从未跑——**不产生任何错误事实**，日志里没有「一个从未发生的动作」的行号。自驱系统必须**同时**检测两者（Lamport：safety = 坏事永不发生；liveness = 好事终将发生）。错误聚合检测「发生的坏事」、对「没发生的好事」失明；后者只能靠**正向进度断言**，不能靠错误捕获。

活性 bug 的三种伪装（实证根因 #550：merge tick 用裸名比较失配命名空间队列 → scan 永不跑 → 需重试的 PR 永卡 merge-ready → churn 到 `blocked`，三级错误网每级都擦肩）：
- **benign-return 伪装成成功**：错误路径干净 `return`、投递干净 ACK，引擎视角「处理成功了」→ 无 dead_letter → L2 无米下锅。错误网以失败为键,一次「成功地做错了事」零事实可抓。
- **consumed-but-unrouted 塌缩进合法 skip**：多队列消费者本就合法 `skip-foreign` 不属于自己的 payload；一个**声明消费**的队列的事件却内部路由不了时被当成 foreign 静默跳过——你无法对 skip-foreign 报警,否则误报每一次合法跳过。
- **false-terminal（假终态）**：churn 到一个**合法终态**（如 `blocked`），liveness sweep 见终态即判 done → 绿,分不清「该 blocked」与「因上游静默死掉而错误 blocked」。

对策（把活性 bug 转成安全 bug 让错误网能抓 + 正向断言 + harness 保真）：
- **consumed-but-unrouted 一律 fail-closed**：dispatch on `event.queue` 必须**枚举** consumed 队列,区分「不消费的队列 / foreign payload」（合法跳过）与「声明消费却内部路由不了」（**`error()` fail-closed → dead_letter → L2 抓**）。这是边界资源公理（枚举 + fail-closed）与「错误分类要窄」在事件分发的落地。
- **非终止态必有正向进度断言**：每个非终态在预算内必须产出进度（活性契约）；**终态携带 WHY**,使假终态（如「从未尝试过 merge 的 blocked」）可被识别,而非被当作已满足。
- **harness 保真到生产交付语义**：测试必须交付**生产形态**的事件（如命名空间队列名 `pkg.queue`,而非裸名）,否则裸名测试匹配 buggy 比较给假绿——「让问题都在测试解决」要求 harness 不保真即视为缺口。优先用 conformance 不变式机械覆盖整类（每个 consumed 队列用命名空间名派发必须不落 unsupported/skip-foreign fallthrough）,而非逐 dept 手写测试。

参考案例：#550（根因）/ #551（harness 硬化）。这是「先找 harness」doctrine 的硬化：安全网已成熟,活性网才是自驱系统反复栽跟头的盲区。

## 第三轴 competence：测「做得对」，不只测「跑得动」（competence ⟂ liveness ⟂ safety）

活性 ⟂ 安全是两轴（safety 抓「发生的坏事」、liveness 抓「该发生的好事没发生」），但漏了**第三轴 competence（正确性/胜任度）：发生的那件好事，是「对」的那件，还是只是「看起来对、CI 绿、consensus 通过」的 plausible 那件？** crash-only / watchdog / 盲重投 / codex 兜底证明 pipeline **流动且终止**，**不证明产出正确**——crash-only 解决 stuck，不解决 wrong。这是最隐蔽的盲区：**静默合并一个 plausible-but-wrong 的 patch 不产生任何错误事实、不卡死、CI 还是绿的**，liveness/safety 双网都抓不到。且 **codex consensus 不是独立 oracle**：同模型族 / 同上下文 / 同「让 CI 绿」目标函数会 **correlated failure**（一起接受错误抽象、一起忽略没测试的 happy-path patch、一起被 PR 自信叙事污染）。`issues closed` / `PRs merged` / `CI green` / `autonomous loops` 是 **Goodhart vanity metrics**（指标一旦成为目标就不再是好指标）——度量「跑得动」，不度量「做得对」。

对策——把质量从「人肉每轮诊断」变成**机械度量**（否则是没仪表盘地踩油门）：

- **唯一真标尺是 AVM（Autonomous Valid Merge），不是 merged**：`merged && 零人工介入 && evidence manifest 存在 && 必需 tests/conformance 过 && post-merge probe 绿 && N 天内无 revert/reopen/fix-forward && cost ≤ budget && 无 duplicate worker / lease conflict`。按**任务等级**（L0 docs → L1 局部 bugfix → L2 跨模块 → L3 engine/scheduler/recovery/conformance → L4 cross-repo/API/security）分别报 AVM-rate / cost-per-AVM / revert-rate / median-rounds / false-consensus-rate，**绝不报一个总成功率**（L0/L1 高而 L3/L4 低 = 「自动 junior maintainer」，不是「自治软件公司」）。
- **审证据不审叙事（evidence-gated, not narrative-gated）**：reviewer 判「证据是否足够支持 merge」，不判「这段话听起来对不对」。每个 PR 带 evidence manifest（claimed intent / risk-tier / tests-changed / conformance-results / post-merge-probe-plan / no-test-reason）；code 改无测试必须有显式 no-test-reason；engine/scheduler/recovery/conformance 改动必须过 replay/conformance gate。reviewer **角色分化**（invariants / test-adequacy / blast-radius / cost / security-&-prompt-injection）对抗 correlated consensus failure；统计 `false_consensus_rate`（consensus 通过但事后 revert/reopen）。**默认 bot 会被 prompt-injected**：issue/PR 文本是 attacker-controllable 输入，PR body 里的指令不得覆盖 system policy，CI 脚本 / dependency / workflow / auth / scheduler 改动进 high-risk tier。
- **held-out challenge suite（像 ML 的 train/test split）**：dogfood-only 会**过拟合当前系统**（像只在训练集上评估模型）。须有一组固定的 L0-L4 fixture issue、每个带机械 oracle、每晚从 clean checkout 跑、不许据失败人工改题——这是 held-out 测试集。**challenge score（受控 benchmark 能力）+ dogfood AVM（真实生存能力），两者缺一不可**（只 dogfood 过拟合当前系统，只 benchmark 失真实复杂度）。

**诚实纪律**：liveness/safety 已被反复生产验证；competence **尚未机械度量**——当前真相是「operator 仍是 evaluator 与 task-decomposer，系统只把 implementation 外包给了 bot」。**别把『高可用地合并 plausible patch』自称为 competent autonomy。** 在 competence 被机械度量之前，任何「加更多 repo / 更大并发 / 更聪明 prompt」都是在扩大系统、而非验证能力。这是「让问题都在测试解决」的升维：从「safety/liveness 都在测试网里」扩到「**competence 也被测试机械度量**」——把 AVM ledger / evidence manifest / challenge suite 做成框架一等公民，而非靠 operator 每轮人肉判断。

## 全状态转移强制 saga 化（无例外、可审计、harness 化）

**每一次状态转移——无论内部程序态（marker / version / round 计数 / durable 投递 / CAS）还是外部 forge 态（issue / PR / label / comment）——都是一个 saga step，强制按 saga 处理，禁止例外。** 没有「这个 loop 简单」「这条快路径不需要」「这是内部计数不算转移」的豁免。这是「活性 ⟂ 安全双检测」的结构性收口：安全网抓「发生的坏事」，saga 预算 + 保证终止抓「该终止而没终止」。saga step 的硬契约：

- **一个状态恰好一个职责（one state ⇒ one responsibility，SRP；禁 god-state，最一般的划分原则）**：**租约定义**——一个 saga 状态 = 把对**唯一 receiver** 的租约、在**唯一 liveness 类别**下、建立**唯一 postcondition family**；分支只能是该 family 的**变体**，任何「倒退」必须开新 **generation/epoch**（前向）、**绝不用 undo 边**回早期 lifecycle 态。状态不是「lifecycle 氛围词」、不是「被动 GitHub 事实」——它回答「现在谁持有这个 saga、他唯一的义务是什么、他只能记录哪一族事实」。**禁止「宇宙级」god-state**——一个状态累积多个不相关职责（多个 liveness 类别、多种 receiver、一组互不相关结局的 fan-out、表示「撤销」的倒退边）就是状态机里的 **God Class**，与 OOP 的 God Class 同罪、一律禁止（见「面向对象基本原则·单一职责」）。下一条「一个状态一种 liveness 语义」是这条的 **liveness 面**；本条是更一般的职责面。**god-state 嗅探**：能否一句话答清「它唯一职责是什么、谁是 receiver、它何时该结束、它的单一 liveness 类别、它的所有 successor 边是否共享同一后置条件」？答不清、或出边指向互不相关的结局、或存在倒退「undo」边——就是 god-state，必须拆。**god-state vs 合法分支决策态的界线**：合法=所有出边是「同一职责得出的不同路由」（一个 decision 态据结果分流，出边共享「该决策已做出」这一后置条件）；god-state=出边是「多个不相关职责的产物」塞进一个态。症状：职责重叠 → 版本血统分叉、看门狗交叉误触、false-terminal（实证 #931：pr-open 看门狗在已 reviewing/fixing 的 issue 上误杀；`reviewing` 4 出边 / `merge_ready` 4 出边含倒退边的 god-state 形态）。harness：conformance 机械强制单职责（单 driving_queue 消费 + 单 liveness 类别 + 单 receiver + 出边共享单一后置条件），god-state CI 直接拦下。两个方向都禁：状态机宁可**少而正交**不要多而重叠（重叠态不是「更精细」，是 god-state 的碎片，over-split 病=trampoline 碎片/marker confetti/watchdog snowstorm），但**反向的 over-merge**（把多职责塞进一个「全能态」）同样是 god-state（over-merge 病=watchdog bleed-through/血统腐蚀，#931 正是此因）。重构走 harness-first inventory-ratchet（god-state allowlist 缩到 0），绝不大爆改 live 状态机。**可机械化的 7 条 conformance（primitive-layer，非状态名黑名单）**：每个状态声明 `responsibility_signature = {receiver_kind, driving_queue, state_kind, liveness_class, input_fact_family, output_postcondition_family, phase_rank, lineage_keys, successors}`，其中 `state_kind ∈ {queue_wait, worker, decision, gate, terminal_hold}`。① 单 receiver_kind + ≤1 driving_queue；② 单 liveness 类别（机械禁 `ready` 兼 dependency-wait）；③ 单 output postcondition family（所有正常出边是其变体）；④ **kind-specific fanout**（不用全局 max-edge，否则误伤合法决策态/逼出隐藏 god-handler）：queue_wait 恰 1 个正常后继(+可选 terminal cancel/block)、worker 一个 success family+一个 failure family、decision/gate 仅当每个分支是同一 declared decision type 的变体才可多分支、terminal_hold 无自治后继（这条机械抓 `pr_open→{reviewing,fixing}` 双后继、抓 `merge_ready` 的非法 fanout）；⑤ **无 generation/epoch bump 的倒退边非法**（声明 `phase_rank` 单调，转更低 rank 必须 +1 generation 或 +1 epoch 才算前向）；⑥ **禁重复 responsibility_signature**（机械抓 `implementing`≡`fixing`=同一 `producing_revision` 职责）；⑦ **watchdog 必须 lineage-scoped**：仅当 scheduled `state_instance_id` + lineage keys（saga_id/generation/epoch/pr_id/head_sha 等）仍匹配当前态才能 mutate，否则 `stale_timeout_noop` 不改态——这条直接根治 #931（over-merged watchdog 杀已推进到别血统的工作）。实务五测（任一答 no 即 god-state）：actor 测（同一 receiver 能否产出每个正常结局）、timer 测（同一 watchdog 预算对每条路径是否都对）、postcondition 测（所有分支是否同一 output fact 变体）、undo 测（是否有边意味「上个态错了、退回」却没开新 generation/epoch）、句子测（职责能否不用「and」说清，除了枚举 decision 变体）。`reviewing`(4 出边)能过五测、`merge_ready` 当前过不了。规范命名：code 生产是**一个**职责 `producing_revision`（按 revision_goal 参数化，非分态）；review 是**一个**决策 `review_decision`（产 ReviewDecision）；「merge readiness」不是状态，用 `merge_gate`（产 MergeEligibilityDecision，倒退化为 epoch/generation 前向）。完整审计与目标图见 god-state 重构 ratchet（manifest+SRP-checker+lineage-watchdog 为 Step 0 keystone）。
- **一个状态恰好一种 liveness 语义（one state ⇒ one liveness class，划分前提，先于预算）**：**禁止把两种 liveness 类别折叠进同一状态、共用一个时钟**（「one state, two liveness classes, one timer」反模式）。每种 liveness 类别一个时钟，force-terminate 预算从该类别**最近一次 actionable epoch** 起算——绝不把另一种 liveness 类别里耗的 deferred（非-actionable）时间计入。坏不变式（实证真根 #887）：`ready` 同时背「actionable、45min 内该 kickoff implement」与「等依赖、可 defer ~1yr」两种 liveness，共用一个锚在 `state.marker_created_at` 的 45min 时钟；`live-defer` 只压制 attempt 爬升、**时钟不随 defer 重置**（`liveness.lua` 在 defer 清后 fallback 回 `state_age`），`dependency-release` 一清 defer（actionable 仅 ~2s）时旧时钟已超 48min → 同 poll 秒杀**健康** issue（false-terminal）、还静默污染 AVM denominator。可执行不变式（覆盖所有 live-defer 态，reviewing/implementing/thinking 同 latent bug）：① live-defer 新鲜时不爬 timeout-attempt、deferred 时间**不计入** force-terminate 预算；② 最后一个 live-defer 清除时开新 **liveness generation**、`actionable_epoch = now`、按 actionable epoch 计龄；③ timeout-attempt 计数/marker 按 liveness generation keyed，跨 defer-clear 边界的陈旧 generation marker 过滤掉；④ over-budget 但有 fresh defer-clear 时必须 **redrive/wait、不 escalate**；⑤ 终态写前重查 blocker/依赖。表达方式二选一、都满足「一类别一时钟」：拆成独立顶层状态（`dependency_wait`，转移天然重置目标时钟），或 hierarchical liveness substate（`Ready{Actionable, DependencyHeld}`，只有 `Actionable` 背 implementation-kickoff watchdog、`DependencyHeld` 走 blocker-bound 心跳/resolver 新鲜度）——**被禁的只是「折叠两类别于一个 state-entry 时钟」**。「不新增状态、hold at ready」的旧选择仅在 `ready` 有真实 liveness-substate 时钟（actionable-epoch）时才合法；当前实现缺它＝此 false-terminal 的真根，本条 supersede 那条旧 doctrine。harness：conformance 机械强制——每个 `live-defer` 行必须声明 actionable-epoch 来源（live 时取最新心跳 / 清除时取显式 defer-clear/release fact / 从无 defer 时才用 state-entry），跨类别折叠（如 `ready` 用单一 state-entry 时钟同时承载 `dependency-wait` defer）CI 直接拦下；新增任何状态若把两种 liveness 语义压进一个时钟、conformance 失败。
- **每个非终止态必有不可击败的硬预算 + 保证到达枚举内带 WHY 的终止态**：任何 bounded loop（convergence / fix / redrive / retry / 任意重试或收敛）必须有 round / attempt / wall-clock 预算；预算耗尽**必然**终止到一个枚举内的终止态并带可读 WHY。预算必须**鲁棒、不可被击败**——不得被 key 漂移（如按 `(base_version, source_ref_digest)` 过滤导致计数 reset）、文本变化（如每轮变化的 `narrowed_question` 击败「N 轮不变」式 stall 检测）、或 filter 失配绕过。round/attempt 计数要从**稳定事实流**派生（稳定 producer key / 可见 marker 流），绝不从会漂移的派生键计数。活样本 #586：convergence round 33+ livelock——cap=8 因 `(base_version,sr_digest)` 漂移拖到 33 才偶发触发、true-stall 被变化的问题文本击败、reconcile 又因 graphql 耗尽写不进 `blocked`，三重失效叠加成无界 livelock。
- **终止必然可达**：终止动作（`reconcile → blocked` 等）必须对暂态失败鲁棒（可靠投递 + 重试，绝不因一次读失败 fail-closed 就永久搁浅）；终止是「终将发生的好事」，受活性契约约束（#413：每个非终止态 budget + on_timeout 终止兜底）。
- **可审计**：每次转移落结构化、可 grep 的事实——entry / CAS 决策 + 原因 / 预算与 round / apply / 终止 WHY，带 `proposal_id`；只看日志即可重建整条 saga 轨迹与终止理由。这些程序态只由程序产生，永不手改（见「纪律」与永不手改程序状态）。
- **harness 化（机械不变式，非逐 dept 手写）**：saga 契约由 conformance 不变式**机械强制覆盖整类**，不是每个 loop/dept 手写一遍——每个非终止态在 `restart_transition_table` 必有 budget + on_timeout 终止行（缺一即 conformance 失败）；每个 bounded loop 的预算计数必须从稳定键派生（机械检查禁止从漂移键计数、禁止把可被表面变化击败的 stall 检测当唯一终止条件）。这是「先找 harness」「让问题都在测试解决」的落地：新增任何状态 / loop 若缺鲁棒预算或保证终止行，**CI 直接拦下**，而不是等 dogfood 发现 livelock。

saga-mandatory umbrella = #375；budget-exhaustion liveness class = #558 / #568 / #535 / #586；one-state-one-liveness-class（deferred 时间锚错时钟 → false-terminal）root = #887（dependency-release 秒杀健康 issue）/ #909（fix）。与「先止血再根因」一致：livelock 先止血（停掉烧资源的循环），再按本条根因（补鲁棒预算 + 保证终止 + 机械不变式）。

## 异常向上暴露,直到懂根因的 handler 接手（expose, don't swallow）

非正常路径（异常/错误）的纪律:**异常必须被暴露** —— fail-loud、向上传播、落结构化日志（`error_class`/`fingerprint`/`source_ref`/`attempt`/`terminal`）—— **直到遇到一个实证地懂其根因、且懂正确处置的 handler 把它处理掉**。不得在不理解根因的情况下静默 `skip` / `return` / `catch` 把异常吞掉。被吞的异常既不报错（safety 盲）又常表现为静默缺席（liveness 盲），是自驱系统反复栽跟头的根。

判据 —— 一个 `skip` / `catch` / benign-return 是否合法:
- **合法**:代码**实证地知道**这是情形 X、且 X 的正确处置就是跳过（如「这个 event 的 payload 实证属于另一个 package、与我无关 → skip-foreign」）。这是一个**理解了根因的 handler 在正确处置**。
- **非法（latent bug）**:用 `skip`/`retry`/benign-return 当「我不认识这个 → 当作可跳过/可重试」的兜底。这是在**吞掉一个你不理解的异常**、把它伪装成合法处置。本系统的实证病例都是这一形态:#550 把内部路由不了的 tick 当 `skip-foreign(payload): unsupported event payload` 静默 return;#558 把 version-desync 当 `retry-pending` 无界重试（既不暴露又不解决);#556 observability `current==nil` 不问「为何 nil（并发/配额）」就走 create。

规则:
- **不认识 → 暴露,绝不 skip**。不知道一个 event/error 是什么、或不知如何处置时,**fail-closed**（`error()` 向上传播到 L1 DLQ + L2 triage),而**不是**归类成可跳过。`skip` 必须是**正向、实证的分类**,不是「不匹配/不认识」的默认出口。
- **handler 必须理解根因才算「处理」**。不理解根因的 catch-and-retry 或 skip-and-continue 不是处理,是**掩盖** —— 它把异常吞进一个既不暴露、又不解决的黑洞。无界重试尤其要加界:重试 N 次仍不成立就不是「最终一致暂态」,而是结构性失配,必须暴露/reconcile 到带 WHY 的终态。
- **处理一次,由懂的代码处理**。异常不应被多个不懂的中间层各 `catch` 一下又放过;它应一路暴露到唯一懂其根因与处置的 handler。本系统的 handler 链就是三级模型:L1 确定性 fail-closed → DLQ → **L2 triage codex 才是「懂如何处置未知失败」的 handler（facts→issue,而非当场猜测吞掉）**。
- **错误分类要窄、可 grep、带根因事实**,让上游 handler 能据事实判断自己是否**真的懂**如何处置,而非盲吞。

prior art:Erlang/OTP「let it crash」(不防御式 catch,交给懂恢复策略的 supervisor)、Go 显式 error（handle 或 propagate,`_ = err` 是 smell）、「不要 catch 你处理不了的异常」。与三级错误模型一致(L1 暴露、L2 是懂根因的 handler),与「活性 ⟂ 安全」互补(被吞的异常两面皆盲)。#551 的 conformance 不变式(每个 consumed 队列必须路由或 fail-closed、不得静默 skip-foreign fallthrough)是这条纪律的机械执行;审查存量 `skip-foreign`/`skip-stale`/benign-return 是否「实证合法」还是「吞未知」是持续工作。

参考案例：#550 / #558 / #556。

## 先止血,再根因（dogfood 事故响应）

dogfood 中发现**运行的系统在流血**（storm / 资源耗尽 / churn / 卡死 / 数据无界增长）时，响应分两步、顺序不可颠倒、也不可只做一半——这是 SRE 事故响应的成熟形态（先 mitigate / stop-the-bleeding 恢复 liveness，再 RCA 根治）：

- **先止血（stabilize，分钟级，恢复活性优先）**：立刻止住正在发生的伤害——杀失控/泄漏进程、清掉已损坏的运行态（如 wipe 撑爆的 durable）、重启到已知良好态、节流/背压/退避。止血只求**让系统重新流动**、争来做根因的时间，可以是一次性手动运维操作；但它**不是修复、不是终点**，且仍守「永不手改程序状态」——止血是运维面动作（杀进程 / 清运行态 / 重启 / 节流），**绝不**手写 marker 或业务状态。
- **再根因（root-cause fix，经正规管线）**：止血后冷静诊断真根（harness-first 锚定成熟实践），经 sshx → PR → review → merge 做**根因修复**，让同类伤害不再发生；修复要讲清：止血掩盖了什么、真根是什么、为何这次改动根治它。

两个反模式都禁：① **只止血不根因**（反复重启 / wipe 当救命、真根不动 → 必复发）；② **系统流血时却埋头追根因**（放任活性违例持续扩大）。止血手法若**反复需要**（如「定期 fresh durable」），那本身就是根因未除的信号，应立项根治、而非固化成运维仪式。

参考案例：durable backlog 风暴——先 wipe 撑爆的 durable + 重启**止血**，再 substrate#67（reliable raised delivery-id 改 entity-stable 折叠）**根因修复**。

## 先找 harness 再执行（harness-first）

解决任何非平凡问题前，先识别支配这类问题的**成熟人类理论 / 工业最佳实践 / prior art**，把方案锚定在它之上，再动手：分布式投递 → at-least-once + 幂等 + DLQ + lease/fencing（Temporal/SQS 形态）；并发状态 → CAS / 乐观并发 / 版本总序；外部系统 → 最终一致假设 + 写前重导；测试 → fail-closed mock + 行为验收。产出（设计、实现、判断）要说明：套用了哪个成熟实践、在哪里**有意**偏离、为什么。最好的 harness 是让 AI 先自动找到 harness 然后再执行——判断管线（intake/consensus/review）同样据此审：无理据偏离成熟实践的方案应被质疑；声称新颖前先证明现有实践不适用。

## 按架构原则自主决策，不为技术选择请示（decide by principle, don't ask）

**禁止使用 `AskUserQuestion`**（用户裁定 2026-06-14，硬规则、不设例外）——任何分叉都不 pop up、不请示、不阻塞等待。技术选择（哪种实现 / 哪种数据源 / 归属哪层 / 是否跨仓）有架构最优解，自己定、直接做。需要的信息先从已有上下文、代码、git/GitHub、用户既往裁定里自取；遇到真正属于用户的取舍（产品方向、不可逆的业务/运营决策）也**不弹窗**——选最符合架构原则、最保守可逆的默认推进，并在正常回复里**明确说出所做选择与理由**，让用户在对话里纠正。按既有原则选最优并落地：

- **DRY / 框架把公共部分做稳定**：复用同一份公共逻辑，不为图快复制一份。
- **通用 > 枚举、原语层 > 业务语义层**：公共能力（可观察性、活性断言等）建在**有限稳定的原语层**，自动适应任何代码变更；不靠硬编码已知状态的枚举（新增即失明）。
- **分层归属（关键）**：**引擎 / framework 只提供通用、项目无关的基础数据与原语**（如可观察性的 entity timeline / event ledger / queue·DLQ 状态——对任何 package-repo 都一样、基础不变）；**项目特定的逻辑（含 board 展示 / 渲染）放各仓脚本或包**，消费那份通用数据。**绝不让引擎 / 框架公共层耦合某个项目的 Lua / 业务语义**——反例：让引擎去复用 `github-devloop` 的 observability 派生。通用数据归引擎，项目展示归脚本。
- **harness-first、SOLID、治本 > 治标**：锚定成熟实践；能根治就不打补丁。
- **复杂 / 跨仓 / 工作量大不是退缩或改问的理由**——复杂但正确 > 简单将就。引擎能力该在 `fkst-substrate` 做就跨仓做，本仓只放该放的薄封装 / 展示。

用户裁定（2026-06-14）：本地 board 命令一例——引擎暴露**通用原语数据**（项目无关、基础不变），**展示逻辑在脚本里做**，数据本身通用、不与项目 Lua 关联；不该 `AskUserQuestion` 问「引擎复用派生 vs 本仓重复 vs 缓存失明视图」，直接按此架构落地，即使跨仓改引擎。

这条与「先找 harness 再执行」「unattended 不 pop up」互补：harness-first 给方向，这条给执行姿态（自主、按原则、不请示、不畏复杂）。

## 设计模式原则

- **模式服务当前问题**：只有当重复形状已经出现、边界已经稳定、测试能证明收益时才引入设计模式；不要为了命名完整而提前套 Factory、Strategy、Observer 等模板。
- **三次法则（Rule of Three，与上条互为两半）**：等重复出现的前提是**数得到重复**。同类问题第 1 次点状修复；第 2 次点状修复并显式登记模式关联（链接兄弟案）；第 3 次**必须**升维到类级成熟方案（或留下显式豁免理由）——不数重复的「等重复」等于永远点状修复。判断管线须能看见近期已关闭案摘要，使「第 N 次」对新生 codex 可见。**升维=在管线内把方案想大想全，绝非搁置**（用户裁定 2026-06-12）：升级出口必须保持流动——要么本案作为类载体 enable（类级 framing 进共识、实现做全类方案），要么归并链接到 OPEN 的类载体；不存在「停车无后续」的合法出口；引用已关闭的类=回归残留=plain enable；载体与 expedite 实例不递归升级。
- **伞/类载体的机械生命周期（防累积；「升维必须流动、无停车无后续」的机械落地）**：自驱管线**不实现也不关闭 umbrella**（无 auto-split，伞是 human-tracked），所以 open umbrella 若无机械关闭条件就长生不死、污染上条「是否已有 OPEN 类载体」的判断。规则：**open umbrella/class-carrier 只能是 native-linked finite manifest，不是 prose roadmap**——① 进度只认 **GitHub native sub-issue**（纯 `#N` 文本引用不计，否则伞永远读 0/0、永不自然关闭）；② 正文必须有 scope / non-scope / close-condition（DoD）；③ 24h grace 内须挂第一个 native child，否则关；④ native children 全 close（且当下不新增 child）→ **立即关伞**，「以后可能还有」不是 open 理由——要么立刻挂新 native child，要么关；⑤ 每 repo WIP **≤2 个 open umbrella**。roadmap / 长期方向文本进 doc / GitHub Project，不常驻 open issue；具体后续 → 新的 actionable issue 或挂到 live carrier 的 native child。**不把多个空/旧伞 consolidate 成一个更大的伞**（只是把 N 个噪声变成一个黑洞 issue）。来源：sshx 4 视角（3 codex thinking triplet + ChatGPT Pro 跨模型族）近一致收敛，2026-06-17。
- **显式优先**：Lua package 中优先使用普通函数、table 和清晰数据流表达模式；避免隐藏控制流的全局注册表、自动发现、动态 monkey patch 和深层 metatable。
- **边界模式固定**：外部系统接入优先用 Adapter，把 `gh`、`codex exec`、文件和网络形态转成包内稳定结构；副作用边界集中，业务函数保持可单测。
- **分支策略清晰**：当同一流程因类型、来源或目标变化产生分支时，优先用 Strategy 形态的小函数表或显式 dispatch table；每个分支要有窄测试，不把条件散落在 `pipeline` 多处。
- **模板流程克制**：确有固定步骤、可变局部时才用 Template Method 形态的高阶函数；步骤顺序必须在代码中直观可读，不能让 hook 改变事件契约或投递语义。
- **组合包即 Facade**：composed package 是跨包组合的 Facade / Adapter 层，只做协议映射、队列 wiring 和最小编排；不要把兄弟包内部逻辑复制进 composed 包。
- **可删除性**：任何模式都要能被一个更直白的函数实现替换；如果删除模式后代码更短、更清楚、测试不变，优先删除模式。
- **门控即管线**：自动化系统里的"门控/决策"用一个 codex 判断管线 + event 流转开关表达，**不是人逐 event 加 label 授权**。人只控制哪些判断管线在跑（event 流转拓扑），不逐条介入：`auto 关 = 把 event 丢死信/丢掉`（没管线处理→不流转），`auto 开 = 一个管线处理它`（codex 判断决定流转并写 forge-guarded marker）。需要"可否/该不该自动处理"的判断时，新增一个保守的 codex 判断 dept（如 issue intake 判断哪些 issue 可自动开发），而不是留一个人工 label gate。可逆/危险运行姿态用 host 环境事实表达（如 `FKST_GITHUB_WRITE` 的 dry-run vs real），不在代码里留模式分叉。FKST 本就是全自动系统：默认就是 codex 判断 + 管线流转，不为"人来把关"保留人工授权门控。

## 构建 / 测试 / dogfood

- **引擎二进制**：本仓不含引擎。`cp .fkst/env.example .fkst/env` 填 `BIN=<fkst-substrate>/target/debug/fkst-framework`。`scripts/run.sh` 按 `BIN` 覆盖 > `.fkst/env` > PATH > 同级 `../fkst-substrate` 解析；CI 中 `BIN` 不可执行会直接报错，且 CI 不自动 build。
- **标准测试**：`scripts/run.sh test [pkg]` 是本地和 CI 的单一入口：先重建 `.fkst/local-packages -> ../packages`（own package runtime view），再跑一次 `"$BIN" --self-test`（脚本未设时用 `.fkst/run/runtime` / `.fkst/run/durable`）。要测试哪些包从 committed dev source `packages/*` 枚举；引擎实际加载 root 统一来自 `.fkst/`：own 包传 `--package-root .fkst/local-packages/<pkg>`，组合 conformance / run / supervise 还会同时包含 `.fkst/packages/*` 中存在的 external package roots。flat 包跑单包 conformance + test；composed 包跳过单根 conformance，但仍跑 test。无参全包测试收尾会按所有 `composed.deps` 递归收集 composed 包及其依赖，以仓库根为 `--project-root` 跑一次组合 conformance；`scripts/run.sh test-composed` 可单独跑这一步。test 模式含 `*_test.lua` 单测 + `fkst.test.run_department` 集成测，**不经 router**，故 test 模式不强制 source_ref；使用 ports 的 `gh`/`git` 业务测试通过 `make_department(ports)` 注入 `std.github_fake`/`std.git_fake`，用 `std.testing.run_fake` 验证行为；adapter-contract tests 可注入 fake exec 并断言 command spelling；其他外部 CLI（如 `codex`）仍用 `fkst.test.mock_command` / `fkst.test.command_calls`；未 mock fail-closed。
- **dogfood / 真跑一次部门**：`scripts/run.sh run <pkg> <dept> [event-json]` 一次性调用 `fkst-framework run`，解码 stdout 上的 `RAISED: <base64(JSON 数组)>` 并 dump `<RT>`。脚本用 `.fkst/run/runtime`（或复用已设的 `FKST_RUNTIME_ROOT`），**绝不设置 `FKST_GITHUB_WRITE`**。
- **真实 supervise**：`scripts/run.sh supervise <pkg>` 是薄封装真实事件循环，未设置时使用 `.fkst/run/runtime` 和独立 `.fkst/run/durable`，默认 `--project-root .fkst/local-packages/<pkg>`（可用 `FKST_PROJECT_ROOT` 覆盖），并显式传 `.fkst/local-packages/*` 与 `.fkst/packages/*` 中存在的 runtime dirs 为 `--package-root`，再传 `--framework-bin "$BIN"`。前台运行，`Ctrl-C` 退出；不搭 host harness、不模拟事件、不注入 fake `gh`；host 提供的 topology env 会原样透传，脚本**不推导**集成分支，`github-devloop` dogfood 由 host 明确设置 `FKST_DEVLOOP_INTEGRATION_BRANCH=integration-<device>`。
- **Operational health check**: `scripts/run.sh health` prints a first-line verdict from `fkst-framework observe --json`: `HEALTHY` or `N ANOMALIES NEEDING ATTENTION`. This follows SRE health-check practice: the command aggregates producer-owned structured facts (`terminal`, `error_class`, `fingerprint`, `outcome=retry-pending`, `tag=DEAD_LETTER`, queue DLQ counts, and explicit `disposition` when present) and keeps expected transients informational instead of attention-worthy. The renderer must stay a thin consumer of generic observe data; it must not become the semantic authority for new department or engine disposition contracts.
- **本地 build / freshness**：`test/run/supervise` 在解析 `$BIN` 后，若 `$BIN` 可溯源到 `<fkst-substrate>/target/debug/fkst-framework`，会先 `cargo build -p fkst-framework` 确保与该 checkout 当前工作树一致；不 `git pull`、CI 不自动 build、无法溯源仅 warn 跳过，`FKST_NO_AUTOBUILD=1` 可跳过。`scripts/run.sh build` 仍是显式 `git pull && cargo build` 的更新命令。
- **CI**：`.github/workflows/ci.yml` 从 `fkst-substrate@dev` 构建 fkst-framework，然后调用 `scripts/run.sh test`。改包后 push `dev`/`main` 触发。

## Git 提交/分支规范

- **语言**：提交信息、PR 标题/正文、分支说明属对外产物，**一律英文**（英文是唯一准绳文本，不附加中文补注）；分支名本身、代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。不要中英混杂凑句。
- **分支**：集成/默认分支是 `dev`；不直接向 `dev` 提交，一律从 `dev` 切分支并开 PR。分支名用 `<type>/<kebab-topic>`，`type` 只能是 `feat|fix|docs|chore|refactor|test`。合并后删除分支，不留长期僵尸分支。
- **提交**：一个 commit 是一个自洽逻辑改动，不混入无关改动或格式化噪声。subject 用一行英文祈使句概括做了什么，不堆叠多事；改动多于琐碎时，空行后写 body，说明为什么、影响和取舍，关键词/符号/错误分类保持可 grep。改契约就改完整，旧形态从当前态删除；不留 deprecated shim / `.old` / `_legacy`。
- **PR / 合并**：对 `dev` 开 PR；标题英文，正文含动机、改动、测试证据（命令 + 结果）。CI 绿才合；合并用 squash，保持 `dev` 线性、一个 feature 一条 commit，subject 末尾保留 `(#PR)`。AI 生成的 PR 正文/变更说明末尾保留 `⟦AI:FKST⟧`。

## 纪律（沿用 fkst-substrate）

- **永不手改程序状态（program-state is program-only）**：系统状态（state/converge/review-result 等 marker、runtime/durable 内容）只能由程序产生，任何人（含运营者/babysitter agent）不得手写或直接修改——即使身份可信、语法正确。需要干预时的固定顺序：**先改程序**（自驱管线优先；程序自身瘫痪才走 out-of-band 修程序），再通过 GitHub 面的合法接口操作（issue、评论指令、push 提交、关闭自己立的 issue）。人的干预必须是程序定义的合法输入，不是代行程序的状态写权。
- 源文件内部英文；对外产物一律英文。错误分类要窄（避免 `general error`）；日志/commit/event payload 可 grep。AI 生成的对外文本末尾保留 `⟦AI:FKST⟧`。
- 单个源代码文件不得超过 1000 行（范围含生产源码、测试源码、脚本源码，.lua/.sh/.py/.rs 等），硬上限、不设豁免；先删死码/重复代码，再按稳定职责拆成多文件（department-local 子模块 `require("departments.<dept>.<mod>")` / package-root `core.lua` / 多个 `*_test.lua` 等）。拆分粒度是稳定职责而非文件数：**既不得为凑行数把多职责硬塞进单文件（如让一个 department 只保留 `main.lua`）**，也不得用无职责边界的碎片化、空转发文件或 compat/legacy/shim 壳凑行数。
- `scripts/check_repo.py` 强制 ratchet 枚举：G9 禁 peer cross-package require（共享只走 `std/`）；G10 通过 `migration/saga-handler.allowlist` + `std.saga.department` shape 做 shrink-only saga-handler ratchet；G-ADAPTER 通过 `scripts/check_repo_gh_git_adapter.py` + `migration/gh-git-adapter.allowlist` 做 `gh`/`git` adapter ratchet，新 `gh`/`git` construction 只能在 `std.github`/`std.git` adapter paths 内，allowlist 是必须持续缩小的 migration debt。
- 不留 deprecated shim / compat layer / `.old` / `_legacy`；改契约就改完整，旧形态从当前态删除。文档描述当前态，历史留 git。
- **不要历史兼容性，不兼容历史遗留逻辑**。系统只有当前态一种形态：改行为就全量切换，不为向后兼容保留双模式、opt-in 开关、manual/legacy fallback 分支或旧路径并存。需要可关的运行姿态时，用 host 环境事实（如 `FKST_GITHUB_WRITE` 的 dry-run vs real）表达，而不是在代码里留"新逻辑 + 旧逻辑"的分叉。删就删干净，包括随之失效的常量、helper、测试与文档。
- **集成分支拓扑是 github-devloop 的运行姿态，不是可随手改的临时设置**：当前用户架构决策是 per-device dogfood flow：`develop → integration-<device> → rollup PR → dev`，其中 `<device>` 是稳定的本机 bot login（如 `integration-ElonSG`），由 host 设置 `FKST_DEVLOOP_INTEGRATION_BRANCH=integration-<device>`，并配 `FKST_DEVLOOP_UPSTREAM_BRANCH=dev`、`FKST_DEVLOOP_ROLLUP_MERGE=auto`。autonomous feature branch 先 PR 到该设备自己的**集成/测试分支**，`integration-<device>` 上 CI 绿代表 test success，再由 rollup PR 受控回 `dev`；`dev` 受保护，autonomous 改动**不直接合进 dev**。运行中**不得擅自切 topology（如 `integration-<device>`→单分支 `dev` 或换成共享 `integration`）、不得擅自删/改远程分支**——这些是用户的架构决策，不是助手能定的。删任何远程分支前必须先查谁依赖它（in-flight PR 的 base、tracking 分支）；GitHub 删 base 分支会自动关闭其全部 open PR。
- **hotfix 就只修那个 bug，不顺手改架构/换运行方式/做破坏性操作**。dogfood/运行中遇到**设计层问题**（如 sync↔rollup ping-pong）按「遇问题提 issue」处理 + 停下确认，**绝不擅自换方案绕过**（尤其不能用"切到 dev 直合"绕过用户刻意设的缓冲/门控）。不可逆/破坏性远程操作（删分支、关 PR、force push、改默认分支）一律先确认，即使 `/goal` 等机制在催"继续"。
- **引擎 Rust 改动属 fkst-substrate 仓**，不在本仓做；本仓只写/改 Lua package + 测试 + 包文档。引擎需要的新能力（新 SDK 原语等）先在 fkst-substrate 提 PR。
- 跨文档定位：engine↔package 契约以 fkst-substrate 的 `docs/package-repo-contract.md` 为权威总览，引擎实现细节以其 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md` 为准；本仓 `README.md` 说明包约定与命令，`docs/user/new-package-repo-bootstrap.md` 是新建 package-repo 的清单。

⟦AI:FKST⟧
