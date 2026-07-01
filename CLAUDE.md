# CLAUDE.md

## 行动前必须逐条通读比对本文件全部原则（HARD GATE·机械前置·无例外·最高优先）

**在做任何非平凡动作之前——诊断 / 根因归因 / 设计 / 实现 / 提 issue / 改代码 / 派 worker / 喂 oracle / 任何破坏性或对外操作——必须先完整通读本文件（CLAUDE.md）的全部原则，把当前计划逐条对照每一条相关原则，确认无一违背；任一条不符就改计划，绝不绕过。** 这是机械前置门，不是建议；不设「这次简单 / 这次紧急 / 我记得大概」的豁免；本门优先级高于本文件其余一切章节（其余章节都在本门内被逐条检查）。

**为什么是 HARD GATE 而非劝导**：本仓几乎每一次事故的形态都是「违背了一条已经白纸黑字写在本文件里的原则」——不是缺原则，是**行动前没把原则过一遍**。凭记忆行动必然出错：记忆会漂移、碎片化，并在冲动下**选择性遗忘那条恰好否定当前冲动的原则**。实证反面教材（必须记住）：把 codex / 外部的随机失败当 bug 去「分类 / 特殊重试 / 建状态」，直接违背早已写明的「核心循环：不分析原因（铁律）+ 程序保持笨 + watchdog 盲重投 + codex 兜底」；在**未核实的前提**上喂 oracle、被放大成一整套精致错设计还 file 出去，违背「第三轴 competence：审证据不审叙事」与「核实数据再建叙事」；健康管线上 out-of-band 双修、越过包边界、手改程序状态、对终态过度反应，各自违背「先止血再根因 / hotfix 只修那个 bug / 守住包边界 / 永不手改程序状态 / 随时可重启—重启永不作为解释」。原则全都在，事故只因未机械过一遍。

**强制动作（每次非平凡行动前，按序执行）**：
1. **通读本文件全文原则**（真读当前文件，不凭记忆——记忆是漂移的不可信源；尤其不要跳过看起来「与本任务无关」的章节，冲动常藏在被跳过的那条里）。
2. **逐条比对**：列出与当前任务相关的每条原则 + 「我的计划符合 / 在哪偏离 / 偏离是否有据」。无据偏离 = 改计划，不绕过。
3. **过三个最常栽的快筛**：① 这是不是 codex / 外部的**随机失败**？是 → 不分析、不建 harness，信盲重投 + 恢复，停手（只有确定性程序层结构 bug 才建 harness）。② 我的叙事是不是建在**未核实的前提 / 别人的归因**上？是 → 先核实每个事实来源（数字单位、字段含义、谁写的、CI/EXIT 等权威反证），再继续。③ 我是不是在 **over-act**（双修 / 越包边界 / 手改程序状态 / 破坏性操作 / 对终态过度反应 / 被 stop-hook/goal 催着走）？是 → 停，回到对应原则。
4. 通过后才动手；中途方案变了，重新过门。

## 实事求是（天津大学校训·认识论根门·HARD GATE）

**一切结论 / 判断 / 决策必须从「实事」——已从源头核实的真值——求「是」，绝不从假设、记忆、愿望（动机推理）、权威之言（包括 oracle、包括用户口述，也要核实）、叙事或惯例求出。** 「实事」= 客观存在、可指到源头的真值；「求是」= 从实事中求真，不从想象中求真。这是本仓（及本会话）反复栽跟头的总根：多次把结论建在**未核实的「事实」**上（误读 `MM:SS` 为 `HH:MM`、`elapsed_ms=429` grep 成 HTTP 429、把别人的归因当根因、#1195 混淆两个 codex），全是用「听起来对的故事」代替「查证过的事实」。

机械门（下结论 / 行动前逐条过）：
1. **每个支撑结论的「事实」都要能指到源头**（哪个文件哪行、哪个 marker 谁写的何时、哪条 CI/EXIT/日志行）。指不到源头的是**假设**，不是事实——标为假设、先去核实，不拿它当地基。
2. **数字 / 字段先验单位与含义再用**（`MM:SS≠HH:MM`、`elapsed_ms=429≠429 限流`、`delivery_id` 段 ≠ 写 dedup）。一个值出现 ≠ 它是你以为的那个含义。
3. **反证优先看、不跳过**：`MSG=framework ok` / `EXIT=0` / CI 绿 / 测试实跑是权威反证，与叙事冲突时信反证、查叙事；30 秒实测胜过一串自洽推理。
4. **别人（含用户、含 oracle）给的「事实」若是未核实的推断，对我仍是假设**——喂下游 / 下结论前自己核实；给 oracle 喂未核实前提 = 制造 correlated 幻觉。
5. **先想最平淡的非-bug 解释**：压力下噪声极易被读成证据；「该状态本就正常」往往比「这是个 bug」更接近实事。

这门**统一并命名**了已有的「核实数据再建叙事」「competence 轴·审证据不审叙事」「BEAUTY GATE·ground-truth 非代理」「HARD GATE 快筛②」——都是实事求是的不同面。违背它产出的东西（错 issue / 错叙事 / 错修复）须整体撤回，不抢救「kernel」。

## 美是真理探测器：解必须忠于本质（BEAUTY GATE·设计质量门）

**工程中的「美」不是装饰，是真理探测器：在一个问题上，最美的解通常就是最对的解，因为工程之美 = 对问题本质的忠实。** prior art：Hardy「美是第一关——丑陋的数学在世上没有永久之地」；Dijkstra「优雅不是可有可无的奢侈，而是决定成败的品质」；Saint-Exupéry「完美不是无可再加，而是无可再减」。本仓实证（2026-06-19，user-as-oracle）：同一个 false-terminal，AI 产出的解（在 marker-年龄这个**代理**上调一个数字 45→120）是丑的、且建在误读上；用户产出的解（「codex 还在执行就 drop 重投」——直查 ground-truth）是美的、一说出来就显然对。差距不在聪明，在**审美判断力**：AI 优化代理，用户要真信号。AI 反复栽的根之一是缺这种美感——抓不到「这个解丑，所以多半错」。

**BEAUTY GATE（接受任何解之前必过；任一「丑的气味」触发 = 停，去找忠于本质的解）**：
- **魔法数字**：解引入/调一个任意常量（为什么 45？为什么 120？）→ 美的解通常**删掉**常量，换成状态检查/结构不变式。
- **代理补丁**：解调一个替身信号（marker 年龄、超时、计数）而非它替代的真值 → 问「这替代的真东西是什么？能直接查它吗？」直查真值。
- **症状分支**：解加 special-case 去**接住**一个失败，而非让该失败**不可表示** → 让非法状态在构造上不可能，而非事后捕获。
- **叙事驱动**：解建在一个故事上（「它一定是 X」）而非已核实的真值 → 先核实再设计（接「核实数据再建叙事」）。
- **缺 inevitability**：你说不出「一说出来就显然只能这样」，它只是「众多合理补丁之一」→ 还没到本质，继续找。
- **跳过目的论**：解没问「这机制**是为了什么**」→ 从目的导出行为（重投是为了激活没在跑的 receiver，故先查在不在跑）。

**关键：自评丑必被作弊 → 美的门是「对抗 + 留痕」，不是自证（这是 HARD GATE，接 competence 轴「审证据不审叙事」）**。让 AI 自问「我的解丑不丑」**必被作弊**——它有强动机答「不丑」并写一段「不丑因为…」的合理化（自评=动机推理），留痕本身救不了，只是记录了那段合理化。**这正是 self-grade 不可靠、本系统一开始就要 consensus / sshx / 对抗 review 的根本原因**；美也必须挂到这套已有的对抗机器上，**绝不新造一个可自证的自检门**。机制三层：
1. **留痕（author 侧，是证据面不是背书）**：产出解后，留下候选 + 显式逐条六气味自查（哪条命中 / 为何都不命中），作为**给对抗者攻击的证据**，不是「我查过了没事」。
2. **对抗（独立视角，默认「丑，直到被证明不丑」）**：由独立 perspective（sshx review triplet / consensus reviewer，独立 context、看不到 author 的合理化）专门找丑，按六条具体气味**查证据**（有没有魔法数字？有没有代理？），author 须用 ground-truth 证据反驳；美只在对抗者**按证据也找不出丑**时通过。丑则打回继续想（留痕可审：第 N 版为何被判丑、改了什么）。
3. **跨模型 + 人兜底**：对抗也会 correlated 失明（同模型同 context 漏同样的丑）→ ChatGPT Pro 作跨模型族对抗，人（user-as-oracle）是最后 backstop（实证：#1195 的丑是人抓到、系统没抓到）。所以这是「对抗 + 跨模型 + 人兜底」的**尽力门，不是完美丑探测器**——诚实，不夸大成自动美判定。

**正向（解之美的判据）**：**忠于 ground-truth（非代理）· 由目的导出 · 无参数/删掉魔法数字 · inevitable · 构造上让 bug 不可能（非事后接住）· 删无可删。** 优先那个「你还想再删却删不动」的解。**这统一了已有 doctrine，不是新增第 N 条**：「make illegal states unrepresentable」（美=构造不可能）、「Harness=唯一规范写法」（美=inevitability）、「核实数据再建叙事」（美=真值非故事）、Occam/SRP/「先找 harness」全是「美=忠于本质」的不同面。

**实证落地：liveness 必须是真实执行状态，不是 receiver 可能无法刷新的自报代理。** watchdog/心跳 doctrine 有个隐藏假设——被监督的 receiver 会周期「踢狗」（自写心跳）。但 **detach/阻塞的 receiver 踢不了狗**（implement codex 阻塞在 `spawn_codex_sync`、detach 跑、无 Lua 循环写心跳），于是「心跳 defer」退化成「从 spawn 起的固定秒表」（`now − started_at`）——一个没有踢狗、只有秒表的假 watchdog，会在 receiver 活着干活时误杀它（实证 false-terminal 根）。新增任何 live-defer 态前先问「这个 receiver **能不能**自己刷新心跳？不能 → 它的 liveness 必须**外部观测**真实执行（进程 / worktree-mtime / 引擎 live-child lease），而非自报 marker 年龄」。且「盲重投」的「盲」只施于**动作**（重发驱动事件、不分析为何慢），**绝不施于前置条件**：重投 / force-terminate 前**必须查 receiver 是否还在执行**（这不是「原因分析」，是 ground state）——在跑 → drop 重投（它唯一目的是激活没在跑的）；没跑 → 激活；没跑 + 激活预算耗尽 → 终止。

## 工作语言

源文件内部一律英文：`.lua`、`.sh`、`.py`、`.rs` 等里的注释、docstring、log/error 文本、模板字符串和标识符都保持英文，与 fkst-substrate 引擎、命令行工具和 LLM 语料一致。例外：明确作为本地化资源表保存的 outward text values 可以使用目标语言的 UTF-8 字面量；这些文本必须保持源码可读、可 grep，禁止用 hex/base64/byte-escape/`string.char` 等 decode helper 隐藏。源文件之外的对外产物（文档、issue/PR/comment、commit message、变更说明）**一律英文**：英文是唯一准绳文本，不附加中文补注/restatement。代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。对话回复跟随用户语言。不要中英混杂凑句子。存量中文文档（含本文件）可保留中文，新增规范性文本英文优先。

## 这个仓库是什么

fkst-packages 是 fkst 的**包库**（"库 B"），承载跑在 **fkst-substrate** 引擎上的 Lua package。引擎本身在隔壁 `fkst-substrate` 仓；**本仓只写 Lua 行为层，不碰引擎 Rust**。

一个 package = `core.lua`（包内共享库）+ `departments/<dept>/main.lua`（department 入口处理器，暴露 `M.spec` 与 `pipeline(event)`；department **不限于单文件**——职责变大时可在同目录拆出 department-local 子模块，经 `require("departments.<dept>.<mod>")` 引入，`main.lua` 只作入口，如 `autochrono` 的 `propose/mapping.lua`）+ `raisers/<r>.lua`（cron/file_watch 触发器）+ `tests/*_test.lua`。包分两类：flat 平包必须自洽、可单根 conformance、0 外部 package namespace 引用；composed 包是一等包，负责组合/适配兄弟包，可引用 `<pkg>.<queue>`，用 `fkst.toml` 的 `[event_deps]` 声明组合 conformance 需要一起加载的兄弟包。当前 flat 包：`packages/github-proxy/`（GitHub issue/PR 入站同步 + 出站评论/label）和 `packages/consensus/`（消费抽象 `proposal`、一个 pipeline 内多角度 codex 共识 + 第 4 个 meta-judge codex 读三角度输出收窄，产出 `consensus_reached` / `consensus_converge`（带 `narrowed_question` + bounded 角度 digest）的通用 source-agnostic 共识引擎）。当前 composed 包：`packages/autochrono/`（消费自有 `issue` → 映射成 `consensus.proposal`，再消费 `consensus.consensus_reached` 产出自有 `reply`，组合 `consensus`）、`packages/github-autochrono/`（组合 `github-proxy` + `autochrono`）和 `packages/github-devloop/`（组合 `github-proxy` + `consensus`，用 GitHub 评论 `state:v1` marker 作为状态事实、`fkst-dev:<state>` label 作为自愈 UI hint，no-consensus 不盲循环也不拆分：`loop` 消费 `consensus_converge` 写 converge-round marker 并带 `narrowed_question` 重发 `proposal` 收窄收敛，router 判 true-stall（round≥3 且连续三轮 question+verdicts digest 不变）时 raise `devloop_reconcile` 交确定性 `reconcile` 部门 drop 到 `blocked`（语义「放弃这个框架」，无 codex、不 split、不升级人）；`ready → implementing` 受 issue 依赖 gate 约束：`core.dependency_gate` 据 GitHub 原生 `issue.blockedBy` 重导依赖、按各 blocker 的 trusted `merged` 判完成，未全满足时 hold 在 `ready`（写 `dependency-wait:v1`/`dependency-cycle:v1` marker + `fkst-dev:blocked-on-dependency` 辅助 label，不新增状态），blocker merge 后下一轮 poll 自动级联放行；环/缺失/跨 repo/blockedBy 截断/gh 失败一律 fail-closed hold（只 satisfied 才放行），gate 同时落在 `consensus_result`/`observe_issue`/`implement` 三处；在 `devloop_ready` 上用隔离 worktree 进入 implementing，失败写入 `impl-failed` 终态 marker；PR 进入 `reviewing` 后复用 `consensus`，通过 `source_ref` / `content_fetch` 回源读取完整 PR diff 与 backing issue 内容做 review decision，`approve` 推进 `merge-ready` 并产生 `devloop_merge_ready`、`reject` 推进 `fixing`，`fixing` 在 `FKST_GITHUB_WRITE=1` 时经写前重导、same-repo PR/head 校验与非 force push 回到新 head 的 `reviewing`；pr-review 的 `consensus_converge` 进入独立 review loop 同样写 review-converge-round marker 收窄收敛，true-stall 时 raise `devloop_review_reconcile` 交 `reconcile` drop 到 `blocked`；`review_meta`（`fix|block` → `fixing|blocked`，无 `accept` 路径、解析失败/歧义 fail-closed 到 `block`、不产 `merge-ready`）不再由 review loop 预算触发，现仅由 `fix` 产出无新 head 时进入；`open_pr` 与 `merge` 没有人工 label gate 或模式开关，唯一姿态开关是 `FKST_GITHUB_WRITE`：未设置只 dry-run，设为 `1` 则直接自治真实写入；`merge` 必须有可信 `review-result:v1 decision="approve"` 与同一 review proposal/dedup/issue/head/version 绑定（唯一 `merge-ready` 与 merge 授权权威是 PR-diff review consensus，`review_meta` 不参与 merge 授权）；`merge` 在 `merge-ready` 或失败重试中的 `merging` + `FKST_GITHUB_WRITE=1` 且 PR open/same-repo/head 未变、存在可信 head-bound `merge-ready:v1` comment-stream review-approval fact、CI green、mergeable 时执行普通 `gh pr merge --merge --match-head-commit`，写 `merging`/`merged` marker 并关闭 issue，缺 gate dry-run，CI 红或明确不可合并回 `fixing`；merge 不使用 GitHub `reviewDecision` / `latestReviews` / `addPullRequestReview`，不使用 admin override，真实全自动运行要求仓库 branch protection required status checks 服务端强制且 bot 不具备 bypass/admin override）。

## 旁路推理通道：nyxid oracle (ChatGPT Pro)

`nyxid oracle` 把推理任务路由到浏览器端 ChatGPT Pro，是 codex / Claude 之外的**旁路推理通道**。用途：复杂任务迟迟不出结果、需要新思路或独立复核时，让 ChatGPT Pro 跑一段独立推理 / 第二意见，再回到本管线决策。子命令、参数、输出字段以 `nyxid oracle --help` / `nyxid oracle ask --help` 为准（已验证子命令：`ask` / `result` / `cancel` / `status` / `attach` / `extract` / `pool` / `sessions` / `session` / `close-session`）。本节只记非显然用法，**不缓存配额/并发数字**：

- **每次先全局搜 pool（不硬编码 pool 名，同「不缓存配额数字」「通用>枚举」）**：每次用 oracle 前先 `nyxid oracle pool list`（+ `nyxid org list`）列出**所有 org 的可用 pool**，从中取 `<pool-slug>`——**slug（非 name）才是 `ask <slug>` 的参数**（如 `chatgpt-pro-pool`，写 `chatgpt-pro` 会 403）。绝不写死某一个 pool 名；新 org/pool 加入后全局搜自动发现，本节不必改。当前账号实测在 **ChronoAI（admin）+ Omega Research** 两 org，各有一个 ChatGPT Pro pool：`chatgpt-pro-pool`（ChronoAI）/ `company-chatgpt-pro`（Omega）——**并行用增容量，一个慢/满就 fallback 到另一个**（ChatGPT Pro 推理本就慢，`dispatched` 数分钟才 `completed` 是常态、未必是卡）。
- **两步异步（省 token，不轮询）**：`nyxid oracle ask <pool-slug> "<问题>" --no-wait --output json` 拿 `task_id`（status=queued）；再 `nyxid oracle result <task_id> --output json` 取结果。status 走 `queued → dispatched(phase=sent) → completed`，`completed` 时 `response` 字段即答案（附 `chatgpt_url`）。单次 `result` 即可，未完成返回中间 status——**不要 busy-loop 轮询**。省 `--no-wait` 则 `ask` 同步阻塞最多 `--wait` 秒。
- **pool 是 org-visibility**：非该 pool 所属 org 成员 `ask` 返回 403（`error_code 1002 forbidden`）、`status` 返回 404。先 `nyxid org join <邀请码>` 加入该 org，再 `nyxid oracle pool list` 才能看到该 pool（看不到即当前账号未加入对应 org）。
- 长 prompt 用 `--file -` 从 stdin 喂，附件 `--pdf`，多轮 `--new-conversation` / `--conversation <id>`；配额与并发（per-user inflight、worker tab 数）以 `nyxid oracle pool list` / `nyxid oracle status <pool>` 实时读出，不在此缓存。
- **本仓 sshx 标配并行 oracle（standing augmentation）**：在本仓做 sshx out-of-band 设计/修复（thinking triplet → meta-judge → implementation → review triplet → fix-or-done）时，**除 3 个 peer-invisible codex worker 外，同时把同一 GoalArtifact/diff 喂给 ChatGPT Pro oracle 作并行第 4 视角**（跨模型族独立推理/复核），与 codex triplet 同 round 派发、各自隔离。oracle 是 codex consensus 之外的**跨模型族交叉验证**，不替代 sshx 的 worker 契约（completion/verdict 仍由 codex envelope 路由,oracle 是 advisory 交叉信号）：thinking 阶段 oracle 与 triplet 并行找根因与设计洞;review/fix 阶段 oracle 复核实现的正确性与遗漏。实证价值（2026-06-17）:#873 false-terminal 的无界-liveness 洞由 codex review triplet 与 ChatGPT Pro oracle **独立收敛**到同一根因;「伞机械生命周期」doctrine 由「sshx 4 视角(3 codex + ChatGPT Pro 跨模型族)」近一致收敛得出。oracle 不可用(403/404/配额耗尽)时 sshx 仍按 codex triplet 正常进行,oracle 缺席只损失交叉验证、不阻断。

## 引擎上下文（写包必须懂；权威完整的 engine↔package 契约见 fkst-substrate 的 `docs/package-repo-contract.md`，引擎实现细节见其 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md`）

- **三级公司**：Company（supervisor + framework + composed graph）/ Department（`departments/<dept>/main.lua`）/ Person（一次 `codex exec`）。不能加层。
- **事件流** `source → fanout → route → spawn → RAISED`：raiser 静态声明 cron/file_watch；Department `M.spec` 静态声明 `consumes/produces/fanout/stall_window`。Department 收到的是 `Event{queue, payload, ts}`，**无生命周期 hook、无共享内存、无持久态**，同一 `pipeline` 跑两次是两次独立调用。
- **SDK surface（固定；权威完整列表与签名见契约文档）**：经原语访问 `raise / spawn_codex_sync / spawn_codex`（`opts.timeout` 控 codex 整体超时，默认 3600s）`/ exec_sync / exec_argv / await_all / with_lock / once / cache_* / git_log_* / count_worktrees / setup_worktree / file / json.decode`（仅 decode）`/ log / now` 等，外加 test 模式 `fkst.test.*`（`mock_command` / `command_calls` / `run_department` 等）。`exec_sync` 是 genuine-shell primitive；`gh`/`git` egress 只经 `forge.github`/`forge.git` 构造 argv 并调用 `exec_argv`。**包不直接碰 `<RT>`/文件系统当状态**——经原语。`once`/`cache_*`/`with_lock` 的 key 是经校验的**可读相对 path**（如 `github-proxy/issue/owner/repo/42`），不是 hex。使用 ports 的业务代码访问 `gh`/`git` 时，不按 command string mock：访问经注入的 `forge.github`/`forge.git` handles，测试用 `testkit.testing.run_fake` + in-process `forge.github_fake`/`forge.git_fake`；`fkst.test.mock_command` / `fkst.test.command_calls` 仍用于其他外部 CLI（如 `codex`）以及 adapter-contract tests（command spelling 本身是被测对象时）。不生成 fake `gh` / `git` / `codex` 二进制；未 mock 的外部命令 fail-closed。
- **事实源 doctrine**：跨 pipeline 的真相只来自 git / 外部源（GitHub）/ 明确 host fact。GitHub 是 eventually-consistent authenticated fact source，不是 strong-consistency KV；读 GitHub marker 当事实时只信本 bot 作者（`FKST_GITHUB_BOT_LOGIN`，真写 `FKST_GITHUB_WRITE=1` 时未配置 fail-closed），用 state marker 的 `version` 做版本有序 CAS，version 总序是 `(updated_at ISO, loop round N, stage_rank)`，同 timestamp 下较大 `/loop/N` 胜过早期 loop，即使早期 marker 阶段更靠后。靠同 issue 统一 `with_lock`、幂等 marker 写入、可靠投递重导和自愈收敛。no-consensus 收敛轮次记在 converge-round / review-converge-round trusted-bot marker，true-stall reconcile 在锁内重导并按 reconcile / review-reconcile marker 幂等跳过已可见的同 round 结果、并 pin 当前 state 与版本段（thinking/reviewing 且 version 段匹配）才落 `blocked`；reconcile 是确定性判（drop→blocked），无 codex，因此不存在两个同 version codex 写出矛盾结果的窗口。包不在源码树或 `<RT>` 存"为活过崩溃"的业务状态；恢复靠 raiser 从源重导 + 下游按 `dedup_key` 幂等。源码树运行期只读。
- **Assignee claim doctrine**：`github-devloop` uses GitHub issue assignees as an optimistic lease and UI surface for multi-instance isolation. The protocol is current-assignees-only: an unmanaged unassigned issue may be assigned to `FKST_GITHUB_BOT_LOGIN`, then re-read before proceeding; any non-self assignee means skip, and every external write re-verifies that the same self-only claim is still held. Losing the claim is stop-on-discovery, while marker trust, version CAS, dedup, review gates, and merge gates remain authoritative. Timeout release is self-only: after a fresh assignee read, the package may remove only its own configured bot login, never a human or non-self assignee; dry-run posture logs the would-release without mutating GitHub.
- **可靠投递 / durable delivery（substrate dev 已合并）**：投递默认可靠，事件经 redb 持久 delivery（at-least-once-until-ack、lease+fencing、retry+backoff、DLQ）。对包作者：
  - **raise 到可靠下游的事件要带 `source_ref = {kind, ref}`**（稳定指针；消费者据此**回源 derive 当前真相**，不信可能过期的 payload；缺失会 fail-closed）。github-proxy 用 `{kind="external", ref="<repo>#<type>/<number>"}`（见 `core.entity_source_ref`）。
  - **【宪法·内容不入 payload】大体量内容（issue body / PR diff / 评论 / 代码 / 文件）绝不整体序列化进可靠投递 payload。** redb 可靠投递 payload 受 ~64KiB 静态上界约束；把内容塞进去 → 被迫机械截断（body 12000 / diff 8000 / digest 600 之类）→ 丢失全貌、codex 看不全、还反复跟 64KiB 死磕（dogfood 实证的反模式）。**内容传输是文件系统 / 网络的职责，不是投递管道的职责。** payload 只承载 `source_ref` 指针 + 小体量控制字段（schema / dedup_key / version / round / 短 digest）；需要内容的 codex / department **据 `source_ref` 从源自己 fetch 完整内容**——`gh issue view` 读全 issue + 全部评论、`gh pr diff` 读完整 diff、worktree / 文件系统读代码、网络读资源——拿全貌、无文字上限。这是「回源 derive 真相」doctrine 的硬化：截断快照本就违背它。历史上的机械文字上限（把内容塞 payload/codex prompt 再截断的 `max_*_len` 设计）一律视为待迁移技术债；**新代码不得新增此类设计**，需要把内容给下游 codex 时，给 `source_ref` + 让它回源 fetch，而非塞进 payload。
  - `M.spec.ephemeral = {"queue"}` 把某 consumed queue 退化成内存 at-most-once；`M.spec.retry = {max_attempts, base, cap}` 调重试，`retry=false` = 失败不重试（仍可靠投递）。
  - **真实 `supervise` 运行需 `FKST_DURABLE_ROOT`**（redb 落点，**不是**可清的 `FKST_RUNTIME_ROOT` scratch）；有可靠订阅却缺它会启动 fail-closed。

## 包结构约定

- **包内共享库放 package-root**：`packages/<pkg>/core.lua`，department 内 `require("core")`。跨包共享只经 declared workspace libraries：`contract`（publishable value/protocol core：`source_ref` / `payload` / `error_facts` / slim `strings`）、`workflow`（saga / env / codex / oracle / registry / sweep / liveness authoring machinery）、`testkit`（test + conformance tooling）、`forge`（GitHub/Git/ports adapters/fakes/debug helpers）、`devloop`（github-devloop product kernel）。package 必须在 `fkst.toml` 声明 direct `lib_deps` 才能 require 对应 library。绑定规则：all `gh`/`git` command construction、shell quoting、execution、stdout parsing 必须在 `forge.github`/`forge.git` argv adapters 后面并调用 shell-free `exec_argv`；package 经 `make_department(ports)` 接收注入 ports，business code 不得构造 raw `gh`/`git` command heads；`migration/gh-git-adapter.allowlist` is empty, and the G-ADAPTER ratchet enforces zero raw `gh`/`git` heads outside adapter paths. production port wiring 由 `forge.ports.install` / `forge.ports.production_handles` 集中负责，不按 department 复制。完整 ports/adapters rationale 与 surface taxonomy 见 `docs/superpowers/specs/2026-06-15-ports-adapters-design.md`。纪律:**禁 peer 跨包 require（A→B 内部，`check_repo.py` G9 强制）；共享只走 declared workspace library deps (`contract` / `workflow` / `testkit` / `forge` / `devloop`)**。这些 libraries 不是包间版本管理 / manifest / 依赖解析。
- **Lua 主仓 vs host 仓的包放置（按语言主属性分，不是不一致）**：本仓是 **Lua 主仓**——committed Lua 源码放根 `packages/<pkg>/`，repo-owned workspace libraries 放 `libraries/<lib>/`，`scripts/run.sh` 生成 `.fkst/local-packages -> ../packages` 作为引擎加载的运行时视图（gitignore）。**网站源码主的 host 仓**若需要 host layout，请直接参考 `docs/adr/0002-host-fkst-layout.md`；本仓不再维护另一套平行 host-layout prose。`.fkst/` 是 **tracked + ignored 混合的「运行时接口目录」**，不是「全 runtime-generated」；本仓（库 B）的 `forge` 对库 B **私有**：host 仓只经 `pkg.queue` 限定名组合库 B 的包，**不跨 require、也不消费库 B 的 forge**；除非库 B 显式把某部分 forge 提升为命名的、带版本的 public 平台 API（届时才经显式 external-lib 机制引用，而非跨 repo symlink）。
- **按稳定职责拆文件，绝不为凑行数把多职责挤进单文件**：department 不必只有一个 `main.lua`——逻辑变大时按职责边界拆出 department-local 子模块（同目录，`require("departments.<dept>.<mod>")`，如 `autochrono` 的 `mapping.lua`），跨 department 复用的才上提到 package-root `core.lua`。`raisers/`、`tests/*_test.lua`、`core.lua` 等其他文件同理：满足 1000 行上限的唯一正解是**按职责拆成多个有边界的文件**，不是把逻辑硬塞进一个文件来「遵守」上限，也不是无职责边界地碎片化。
- **flat 包 vs composed 包**：flat 包必须自有契约、自有裸名队列、0 外部 package namespace 引用，并通过单根 conformance；composed 包可以引用兄弟包 namespace 做组合/适配，但必须在 `fkst.toml` 的 `[event_deps]` 声明所组合的兄弟包，并经组合 conformance 验证。`[event_deps]` 是测试组合的最小约定，不是版本/依赖解析 manifest，也不是部署配置；这是本仓为了让组合 glue 成为 CI 覆盖的一等包而接受的取舍。
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

**守住包边界：新功能默认放包外，能在包外实现就不往已稳定的包里塞（prefer-out-of-package；上面 SRP/OCP/合成复用 在包边界的落地）。** 加一个新功能时先问「它能不能作为独立包 / 包外模块实现？」——**能，就别塞进一个已稳定、职责已收敛的包**。每一次往稳定包「顺手加功能」都是 SRP 侵蚀：它的变更原因变多、blast radius 变大、滑向 **god-package**（等同 god-class，见上「单一职责」与「全状态强制 saga 化·禁 god-state」）。判据（标准 OOP）：① **SRP**——新功能若有自己独立的「变更原因」（不同的 source / 不同的信任域 / 不同的生命周期），它就是独立职责、该独立包；② **OCP**——对扩展开放、对修改关闭：用新包 / 组合**扩展**，而不是改稳定包的内部；③ **合成复用 > 塞入**——composed 包（Facade/Adapter）把兄弟包接起来，不把逻辑复制 / 塞进彼此。本仓实证范式：`github-external-pr-intake`（外部 PR 桥接）、`github-ratchet-migration-slicer`（把切片器从 github-devloop 抽出）都遵此——`github-devloop` 只守「issue→PR→review→merge 生命周期」**这一条**职责，切片 / 外部 PR / 关系 auto-fill 一律落在包外。**边界（不与「模式服务当前问题 / 三次法则」「over-split 也是病」冲突）**：这条治的是**职责归属**（独立职责该放包外，而非默认塞进现有包），不是叫你为单个功能提前造投机抽象、也不是无职责边界地碎片化；**真有独立「变更原因」才独立包，没有就别为「干净」硬拆**——over-split（包/状态碎片化）与 over-merge（god-package/god-state）同为病。

## 包间走前门（发布 seam·迪米特@边界）；及「无穷违例不可能逐个手写 harness」的总解（HARD GATE）

**（一）边界交互原则：一个 saga / 包只驱动它自己的队列；跨包交互只能走对方发布的 seam——对方声明 consume 的入口队列，或它写、你 poll 的 marker——绝不能伸手 `produce` 兄弟包的内部生命周期队列。** 这是「面向对象基本原则」的**迪米特 / 封装 / 低耦合 / SRP** 落在包边界**交互面**上（「守住包边界」治**职责归属**：谁该独立成包；本条治**交互形态**：包与包之间怎么说话）。把包当对象：它的阶段推进是内务，外部只能经它的**公开接口**（consume 的事件 / poll 的 marker）与它交互，不能翻窗户进去驱动它的状态机。「路由」（据 entity / marker 决定 raise 哪个下一阶段队列）必须**和被路由的部门同包**（高内聚）；跨包 push 对方内部阶段队列 = 破封装 + 高耦合 + **god-router**（一个部门背了「观察自己 + 驱动别人」两个职责）。实证（2026-06-21，user-as-oracle）：拆 `github-devloop-pr` 时 worker 把 PR 部门搬进 -pr，却把 PR 阶段路由留在 issue 侧 `observe_issue`（它 `produces` 了 `github-devloop-pr.devloop_reviewing/fixing/merge_ready/...`，**从 issue 侧 push 驱动整个 PR saga**），与 -pr 反向 push `devloop_decompose` 织成跨包环；**composed test 全绿**、单根 conformance 也没拦——只有跨模型对抗 review + 用户直觉点破。正解：-pr 用自己的 `observe_pr`（本就 consume `github_entity_changed`）**自驱**自己的阶段，seam 退回 marker/poll + 对方发布入口。**边界（不与「composed 包可引用兄弟 namespace」冲突）**：composed 包**可以**引用兄弟包——但只能走其**发布的 seam 入口**（Facade/Adapter 该做的），不能引用其**内部阶段队列**；区别就是「走前门」vs「翻窗户」。

**（二）「这么多 case，不能每次手写 harness」的总解**——按「Harness 本质」的强度梯度，**手写 per-case scan 是最后兜底、不是主力；主力是「通用捕手」+「通用预防器」**：

1. **通用捕手 = 对抗 review 把原则当透镜用（零 per-case 代码、规模无限）。** 不为每种违例写一个 detector。competence 轴的对抗 review（跨模型 + 人）本就用「这是否违反 迪米特 / SRP / 封装 / 低耦合 / make-illegal-states-unrepresentable？」这种**通用透镜**抓**新形态**——本 bug 正是这么抓到的（review + 直觉，不是 scan）。所以「无穷多 OOP 违例」的**第一道线是带着原则的 reviewer，不是 N 个检测器**；这条**自然 scale**。
2. **通用预防器 = 把整个「类」提升成一个引擎原语（PREVENT，让该类不可表示）。** 当违例是一个**类**（非一次性），别写 scan——投资**一个**引擎能力 / 类型把整类杀掉。跨包这一类**有两半，要分清**（2026-06-21 核实 substrate 源）：**第一半 `raise(q) ⊆ produces` 引擎已实现**（`crates/fkst-framework/src/raise.rs` 的 `ensure_allowed`：raise 一个不在 `M.spec.produces` 里的队列即运行时 fail-closed 报错）——但它只挡「你只能 raise 你**声明过**的」，**不挡「你能声明什么」**，所以它**挡不住 god-router**：E2 的 `observe_issue` 把兄弟内部队列 `github-devloop-pr.devloop_reviewing` **声明进了自己的 produces**，于是 `raise⊆produces` 高高兴兴放行。**第二半 `produces ⊆ 自有队列 ∪ 兄弟包发布 seam` 引擎也已实现（2026-06-24 核实 substrate 源；此前本节记的「缺第二半 / 无 published-seam 概念」已漂移，据实更新）**：包用 `M.spec.published_seam` 标注哪些 consumed 队列是**公开入口（published seam）**，`crates/fkst-framework/src/supervise/graph_scan.rs` 的 `validate_cross_package_produces` 把每个 department / raiser 的 produces 限制成「自有命名空间 ∪ 某兄弟的 published_seam」、声明兄弟**内部**（未发布）队列即 `bail!`（"produces sibling queue … which is not published by that package in M.spec.published_seam"）fail-closed；`validate_published_seam` 另校验「publish 的必须是自己 consume 的」。该 graph 校验在 conformance 路径也跑（`host_conformance.rs` 的 `load_host_graph_for_conformance(&roots)`），故对任意 package roots 生效、随包走、不看 repo 身份。于是完整链 `raise ⊆ produces ⊆（自有 ∪ 兄弟 published_seam）`已**构造成立**——「翻窗户 produce 兄弟内部队列」现在**写不出来**（conformance/CI 红、零 scan、一个原语挡无穷 case）：E2 的 `observe_issue` 把 `github-devloop-pr.devloop_reviewing` 声明进自己 produces，现在会被 graph-scan fail-closed。这正是「框架做稳定公共部分」：**边界强制归引擎**（已落在 fkst-substrate，不在包侧硬造）。
3. **per-case scan = 仅迁移兜底，其泛滥本身是 smell。** scan 只用于 (a) 原语落地前的迁移期（allowlist→0）、(b) 廉价兜住一个**已知会复发**的形状。**若在为每个 case 写 scan，就是在 ① 苟且、本该投资 ④**——正是「Harness 本质」的警告。现有一堆 G-scan（G-ADAPTER / G-DEDUP / …）凡能落到 ②③④ 的，都欠一次「提升到引擎原语」的债。

**纪律（把三层串起来，每抓到一个违例就过）**：问「**这是一次性，还是一个类？**」一次性 → 它是 review 透镜的发现，**修了走人、不建 harness**；一个类（三次法则 / 明显可泛化）→ **提升到最通用的 PREVENT 原语（引擎能力 / 类型），不是再加一个 scan**。所以「不能每次手写 harness」的答案是：**本来就不该每次手写——通用捕手是带原则的 reviewer（免费 scale），通用预防器是少数几个引擎原语（杀整类），per-case scan 只是迁移脚手架、要尽量少。** 这统一了既有 doctrine（迪米特 / SRP / 低耦合 + 守住包边界 + 信任契约 + Harness 本质 + competence 轴），非新增第 N 条——是它们在「**如何不靠逐例手写就守住原则**」上的同一张脸。⟦AI:FKST⟧

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

**部署即重启、随时可重启：`supervise` 必须能在任何时刻被 SIGKILL + 重启而不丢工作、不造成永久停滞。** 这是 crash-only software（Candea & Fox，见上一节）的硬契约，不是「尽量」。系统不做 drain / 优雅关停 / 在途排空；恢复靠两条既有机制：① **durable 投递**（redb at-least-once + lease/fencing + retry）让在途事件重启后续投；② **从 marker / git / 外部源回源 re-derive**（真相不在内存态）让任何中间态被重新推导、重驱。**重启 orphan 在途 codex，不是杀它**：`dogfood.sh` 重启 = `kill -9 <supervise_pid>`，只杀 supervise 进程；在途 codex 是其子进程（supervise → department-child → `spawn_codex_sync` → codex），SIGKILL 不向下传播，故 codex 被 **orphan（reparent、继续跑到完成）**——不是被杀（实证 2026-06-19：一个 implement codex 启动早于重启后的新 supervise、仍存活）。重启真正中断的是那个**同步 department 调用**（department-child 也被 orphan，但它要 RAISE 给的父 supervise 已死、事件丢失）；新 supervise 从 marker re-derive、按 live-defer 心跳变陈**重驱**同血统 codex——orphan 存活 + 重驱可形成**短暂 double-spawn**（#1101 类），由 version-CAS + dedup marker 幂等收口。所有工作幂等、可重入。**纠错（2026-06-19，user-as-oracle）：「重启杀掉在途 codex」是错的——codex 不被杀，只是 orphan；以前这么说/这么写都属误判。** 重启因此是**无害的常规运营动作**（部署新代码、清运行态、换 BIN），随时可做，不需攒批次、不需等"安全窗口"。

**铁律：重启永不作为问题的解释。** 看到重启后某 strand 没进展时，**默认归因不是「重启 churn 掉了它」**——这是违背本契约的偷懒归因，会掩盖真缺陷（活性盲区）。crash-only 下重启理应被 durable + re-derive 吸收；若重启**确实**导致永久丢失/停滞，那必然是一个**活性契约缺陷**（durable 没续投、re-derive 没重导、或「心跳变陈 → re-spawn」链断了），要 root-cause + 提 issue，绝不用「重启影响了它」搪塞，也绝不为「避免 churn」去不重启 / 攒批次（那让进程长跑陈旧代码，反害——见 dogfood「立即重启别攒批次」）。运营随时重启；把工作活下来是**系统的责任**，不是运营的小心翼翼。实证（2026-06-17）：误把一个 fixing-loop 停滞甩锅给「我反复 restart churn 掉 fix codex」，实查发现重启后 fix codex 已被正常 re-spawn（crash-only 生效），真信号是另一处 marker-visibility version-desync——偷懒归因差点掩盖真缺陷。

**Codex 并发由引擎 admission 控制（默认全用，dogfood 不 override）**：`FKST_CODEX_PERMIT_SLOTS`=**20**（全局 codex 进程许可池上限）、`FKST_MAX_IN_FLIGHT_PER_DEPT`=**16**（每 department 并发 durable child 上限）、`FKST_DURABLE_ADMISSION_BURST_PER_DEPT`=**1**（每 dispatch pass 每 dept 只准入 1 个新 child——缓启，#512 thundering-herd 后特意设的稳态保护）。**实测同时在跑的 codex 数（常 3-5）远低于 20 cap，是「当下需求 + burst=1 缓启」而非被限流到顶**——别把低 codex 数误读成 admission 卡死或 cap 太小（纠错 2026-06-19，user-as-oracle：此前误判 cap≈3，实为 20）。想让排队的工作铺得更快可调高 `burst`，但 burst=1 是特意的 herd 保护，改前权衡（herd 风险 vs 铺开速度）。

## 信任契约,别在包层重造框架已保证的东西（「下游不稳定」是错误前提,是意外复杂的根）

**意外复杂几乎都长在同一个错误前提上:「下游 / 引擎 / 兄弟包可能不可靠,而我没法确知,所以必须自己兜底。」** 一旦默认周围不稳,就会在每个边界叠防御——自造 durability、persist-before-ACK、双保险、对已保证的东西反复重验——复杂度就是这么长出来的。`github-proxy` / `consensus` 之所以简单,正因为它们**不**做这个假设:信任拿到的契约、只管履行自己的契约。`github-devloop` 更复杂,很大一部分就是这个防御性前提的累积,而非某个具体设计本身难。

**铁律:假设契约成立来写包(于是包极简);若发现契约被违反,去它所在的层修(通常是 engine),绝不在下游叠 paranoia 去绕。** 这是「框架做公共稳定部分、脚本写最简单业务」的同一句话,也是「异常向上暴露,直到懂根因的 handler 接手」在信任维度的对偶:契约破了就让它暴露到该修的那层,不在包里悄悄兜。

**精确切一刀(实事求是,别从「过度防御」滑到「过度信任」):**
- **该信任、却在防的(假前提,删):** 引擎可靠投递会不会丢、下游包会不会不写、该来的回调到底来不来。这些是**契约**,它们成立——包不得在自己这层再造一遍(自建 durable marker、persist-before-ACK、把出站 outbox 当可靠层用,都是这类冗余)。
- **真实的契约属性(要处理,但这不叫「不稳定」,且框架已解决一次、包只管信任):** GitHub 最终一致(读滞后写)、crash-only(进程会重启)、at-least-once(同一事件可能来多次)。框架用可靠投递 + 从源 re-derive + version-CAS 幂等**解决一次**;包**信任**它,只履行幂等——这正是「多次无所谓」。**version-CAS 幂等不是防御、是幂等机制,留;persist-before-ACK 之类是防御、删。**
- **框架真有缺口时:** 如 #1101 的 `raise()`→emit 窗口(raise 只进 in-process buffer、pipeline 返回后才落持久队列,这中间崩溃会丢)——那是 **engine 去补、让契约真的成立**,不是包叠防御去绕。

**架构形状:业务逻辑完全信任,安全只集中在唯一一个诊断兜底里。** 按**完全信任**写业务(happy path 信任契约 / 事件回调,零 per-op 防御——这是最简形态);把所有「万一有 bug 让某契约没被履行」的担心**集中到唯一一个全局诊断**——一个 level-triggered sweep,poll「哪些态没按契约推进(该进展却没进展)」,对它们**盲重投**(零根因分析、version 单调 +1)。**理论上它永不命中**(契约成立时无人 stuck);**一旦真有 bug 让某步没履行契约,这一个 sweep 就能恢复。** 这正是本仓既有的 watchdog 心跳盲重投 / liveness sweep(见「核心循环」「活性 ⟂ 安全」「全状态强制 saga 化」)——本条点明其定位:**安全只许活在这一个 sweep 里,别散进每个操作的 per-op 防御**(persist-before-ACK、自建 durable marker 就是这种散落,删)。

**于是 poll ≡ 消息激活(事件回调)只是触发同一个幂等 reconcile、等价。** happy path 可**完全信任**事件回调(最简);那唯一的 poll-诊断是兜底、不织进每个操作;两者都靠「reconcile 幂等(多次无所谓)」成立,绝不带 must-not-lose 语义。诊断这条信任链最稳的形态是 level-triggered——每轮从持久事实源(GitHub marker / git / 外部源)重新推导,完全不依赖任何一次事件投递是否成功(连发射端 raise 窗口的丢失都自愈)。本仓既有范式:`dependency_wait`(父等子 = blockedBy gate)就是 poll 驱动(`driving_queue = github-proxy.github_entity_changed`)、每轮重读依赖态、幂等推进 ready/blocked——任何「父等子」态(含 `awaiting-pr` 等 PR 子终态)都照这个统一形状,而非发明 push 专列 + 自建 durability。

**与「边界资源公理」互补、画同一条线:** 真正不服从内部哲学、需要枚举 / 中介 / 计量 / 预算的,只有**真·外部边界**(GitHub、外部资源),且由框架在**一处**中介掉;边界之内的 engine↔package、package↔package 契约一律**信任**,别把边界的 paranoia 往里蔓延。防御只打一次、打在真边界上;里面越信任越简单。

**纠错(2026-06-20,user-as-oracle):** 拆 issue/PR saga 时我纠结的 persist-before-ACK、「terminal 绝不能丢」、raise 窗口、子状态读不到——底层全是这个「下游可能不稳、我得自己兜」的假前提;user 一句「这个前提就是错的」点破。按本条:那套 push durability 全删,`awaiting-pr` 做成 `dependency_wait` 的孪生——**业务完全信任(可信事件回调),安全交给那唯一的 liveness sweep 兜底**(完全信任 + 一个诊断盲重投:理论上不命中,有 bug 也能恢复)。⟦AI:FKST⟧

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
- **重构不改语义（refactoring is behavior-preserving by definition——否则就不是重构，这是基础哲学）**：refactoring（Fowler）改的是**结构**（职责划分、状态命名、代码组织），**绝不改可观察行为**（同输入 → 同 effects / 同转移 / 同升级路径 / 同终态 / 同投递）。**一次改动一旦改了语义，它就不是 refactor，是 behavior change**——必须当 behavior change 单独论证 + 单独 review，不得披着「重构 / 拆 god-state / 声明 signature」的外衣偷偷改路由、改升级、改终态。god-state ratchet 尤其守此：移除一个 god-state 的 fused 边，必须把那条边的行为**等价重路由**（同条件 → 同可观察结局，只是经更干净的结构），绝不静默换成别的结局；很多时候根本不必移除边——只需**正确分类**它（如把一条边声明为该 worker 的 **failure 出边**而非删掉），就既单职责又零语义变化。实证反例 #935：打着「fixing 是 code-producer、不该 decide review-meta」移除 `fixing→review-meta`，把 fix-failure（no-new-head/no-fix/codex-failed）从原本的 `→review-meta→fix|block` 确定性升级**改成** `→reviewing` 重审循环（更贵、更慢、且重审未变的已-reject head）——这是 behavior change 伪装成 refactor，对抗 review 只验了「loop 有界」却漏了「目标语义变了」。正解（零语义变化）：把 `review-meta` 声明为 fixing 的 **failure 出边**（`reviewing`=success / `review-meta`=failure 的干净 worker），保留原升级语义、同时过 grader。机械测：每个重构 PR 必须能论证「**无可观察行为变化**」（同输入同 effects/转移/终态/投递）；论证不出就不是重构——标 behavior-change、单独 review、绝不计入「重构 ratchet」。与「hotfix 只修那个 bug、不顺手改架构」「先找 harness」同源；这条是 god-state ratchet 每一步的硬前置。
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

## 直接改根因，不畏复杂（deferred cost 10–100×，HARD GATE）

**发现问题就直接修它的根因——不管根因多复杂、跨多少仓、工作量多大；绝不为「这次便宜」选 band-aid / 症状补丁 / 把已知真根推后。** 成本会**复利**：一个 symptom-patch 留在原地，日后代价是当下的 **10–100×**——它复发、催生补丁去补补丁、掩盖真缺陷让安全/活性/competence 网都抓不到、与别的债叠加、且越晚越贵（越多代码依赖那个坏形状，改它的 blast radius 越大）。「根因看起来贵」从来不是推迟的理由：根因只会越来越贵，band-aid 的「省」是负利率借贷。prior art：**1–10–100 缺陷成本律**（同一缺陷，设计期修 1 / 集成期修 10 / 生产期修 100）、**技术债利息**（Cunningham：债不还，利息吃掉你）、**broken windows**（一扇没修的破窗招来更多破坏）、**shift-left**。本仓实证病例：把内容塞 payload 再机械截断的 `max_*_len`、用 retry-tuning 掩盖 write/read causal race、用 per-case 确定性分支堆砌而不上 harness——全是「当时省事、之后 10–100× 偿还」。

**这是 HARD GATE，不是劝导文（机械强制、CI fail-closed，非 prose）。** 与「Harness 的本质：唯一写法 + 机械禁旁路」同形——band-aid 就是要被 ratchet 到 0 的**旁路写法**：

- **band-aid 不是免费选项，必须登记为 shrink-only 债 + 链接根因 issue**：确需先上临时补丁（如止血）时，它必须进一个 shrink-only `migration/<class>.allowlist`（或等价债账本），带一行 WHY + 一个**根因修复的 tracking issue 号**；CI 在账本**增长**（新 band-aid 未登记）或补丁**无链接根因 issue** 时 fail-closed。这把「我先 band-aid、之后修根因」从口头承诺变成机械收敛：债可见、ratchet 向 0、不静默累积（静默累积正是 10–100× 复利的发生方式）。
- **已命名的 band-aid 反模式在新代码里 conformance-禁止**：内容截断 `max_*_len`（正解＝`source_ref` 回源 fetch）、retry-tuning 替代 causal-ordering（正解＝写确认因果 outbox）等，新增即 CI 红，存量进 shrink-only allowlist 向 0。
- **新症状类必须建自己的 shrink-only ratchet**（detect→prevent 梯度，见「Harness 的本质」）；现有 G-ADAPTER / G-DEDUP / forward-direct-raise / saga / god-state / liveness / coverage 就是这条已落地的实例，新类照办，绝不只在 review 里口头提醒。

**边界（不与既有 doctrine 冲突）**：与「先止血，再根因」一致——止血是**临时止血带**（运维面、争时间），本 gate 强制其「再根因」那一半（止血落账本 + 挂根因 issue，绝不固化成仪式）；与「hotfix 只修那个 bug，不顺手改架构」一致——改的是**这个问题的因果链根因**，不是借口去重写无关模块（根因 ≠ scope-creep）；与「模式服务当前问题 / 三次法则」一致——修根因 ≠ 提前造投机抽象（10–100× 说的是**放着已知真根不修**的代价，不是「没预先泛化」的代价）。

## 先找 harness 再执行（harness-first）

解决任何非平凡问题前，先识别支配这类问题的**成熟人类理论 / 工业最佳实践 / prior art**，把方案锚定在它之上，再动手：分布式投递 → at-least-once + 幂等 + DLQ + lease/fencing（Temporal/SQS 形态）；并发状态 → CAS / 乐观并发 / 版本总序；外部系统 → 最终一致假设 + 写前重导；测试 → fail-closed mock + 行为验收。产出（设计、实现、判断）要说明：套用了哪个成熟实践、在哪里**有意**偏离、为什么。最好的 harness 是让 AI 先自动找到 harness 然后再执行——判断管线（intake/consensus/review）同样据此审：无理据偏离成熟实践的方案应被质疑；声称新颖前先证明现有实践不适用。

## Harness 的本质：唯一确定一种写法，机械禁止其他写法绕过（one canonical way, bypass forbidden）

**一个 harness 的核心不止是「锚定 prior art」——而是：对「做某件事」唯一确定**一种**规范写法（the single canonical way），并用机械不变式（conformance / ratchet）让其他一切写法「不可表示」（CI 直接拦下）。** 这是「make illegal states unrepresentable」从「非法状态」推广到「非法写法」：不是写文档劝大家用 X，而是让 X 成为**唯一能通过的表达**，任何绕过 X 的旁路写法 Y 在 CI 红、合不进来。

**它根治的失效形态——多写法并存 ⇒ 某种写法绕过了安全/正确的那种 ⇒ 静默 bug。** 旁路写法通常「能用」（不报错、CI 绿、看起来对），所以**只有机械不变式抓得到它**：人 review 看不出「这里本该走 A 却抄近路走了 B」，因为 B 单独看也合法。系统反复栽的根就是这个——同一目的存在 ≥2 种写法，迟早有人（codex 或人）走了那条不安全的旁路。**典范病例（写读 marker-visibility 竞态）**：一个 state-trigger 有多个 producer——安全的因果路径（写确认 → comment_handoff 派生 trigger）+ 不安全的旁路（转移 dept 与自己的 GitHub 写**并发**直 raise trigger，同 dedup_key 抢赢、顶掉因果 raise）。bug 的真身不是「重试太慢」，是「**有唯一安全写法却被另一种写法绕过**」。Harness 解 = 唯一确定「forward trigger 只能由写确认因果派生」+ conformance 枚举断言该 trigger 队列的唯一 forward producer，旁路的 forward-direct-raise 一律 CI 红（redrive/self-heal 的「读到已可见 marker 再重投」是另一种**显式区分**的合法写法，机械区分、非一刀切禁）。

**这正是本仓所有 ratchet/conformance 的同一形状**，本节给它们命名共同本质：G-ADAPTER（gh/git 只能经 `forge.github`/`forge.git` argv，裸 gh/git=0）、G-DEDUP（一份 canonical body，禁字节级 clone）、强制 saga（唯一形状 `workflow.saga.department`）、god-state（一状态一职责）、活性契约（每个非终止态必声明 budget+watchdog）、ports（唯一 egress 路径）——全是「唯一确定一种写法 + 机械禁止旁路」。

**机械实现「唯一写法」的强度梯度：PREVENT > DETECT，scan 是兜底不是终局。** 建 harness 先问「能不能让旁路**根本拿不到原语 / 发不出 effect**」，而不是先写 grep。按旁路性质选档，强→弱：

- **④ capability restriction（最强：旁路原语不在业务代码 reach 内）**——可绕过的 primitive 只注入给 canonical path，业务层够不着 → 旁路**写不出**。判据：旁路 = 调用某**共享原语**。已实现例：`forge.ports` 持有 `gh/git` argv 构造权，业务 dept 拿不到裸 gh/git。目标形态：`M.spec.produces` 成为 `raise()` 的**能力授予**（dept 只能 raise 自己声明的队列）。
- **③ runtime guard on schema grant（动态语言里的实际 PREVENT）**——effect 边界按 schema 授权，未授权 **fail-closed**：旁路语法还能写，但 effect 发不出。判据：旁路 = 需授权的 **effect**。如引擎强制 `raise(queue) ⊆ 当前 dept 的 produces`，未声明即 fail-closed（且 raised 事件传输须不可被业务代码伪造，否则 guard 可被绕）。
- **② declarative schema / typed table（结构上不可表达）**——契约写成**数据**，只有 canonical 形状有字段，缺字段 / 多 producer / 缺 liveness 行直接 conformance 红。判据：旁路 = **数据的结构属性**。已实现例：`restart_transition_table`、`workflow.saga.department(spec, handlers)`、responsibility signature、liveness contract。**别为此造 YAML / 外部 DSL——`M.spec`/Lua table + conformance 本身就是 schema-checked DSL**，加序列化边界只增成本不增强制。
- **① conformance scan / ratchet（DETECT，仅迁移期 + 防回归兜底）**——grep/AST 抓**已知**旁路形状，allowlist shrink-only 到 0。旁路仍可写、false negative 永远可能。判据：旁路 = emergent 跨文件 / 任意源码属性（字节级 clone、文本 smell），天然降不到 ②③④。

**铁律：能表达成 ②③④ 的契约不得长期停在 ① scan。** scan 的职责是**暴露迁移债 + 防回归**，不是权限边界——把它当主防线，就是「编程语言太灵活、谁都能写另一种写法」的根源。判一个旁路该爬到哪档：调用共享原语→④收紧原语（如 forward-direct-raise 应经 produces 能力化，而非永久停在 scan）；数据结构属性→②schema；需授权的 effect→③runtime guard；任意源码 emergent→①scan 兜底。注意粒度：「一队列一 producer」只施于 **lifecycle/authority 队列**，telemetry/fanout/shared 队列可合法多 producer；同一队列若有 forward（写确认后触发）vs redrive（读到已可见 marker 后重投）两种权限语义，队列级能力分不出二者，需 queue 拆分 / typed egress 或保留 schema+scan backstop 区分。引擎层能力化（`raise⊆produces` fail-closed + 不可伪造的 raised 传输）属 substrate，不在包侧硬造。

**纪律**：建 harness 时，问题不止「prior art 是什么」，更是「**这件事的唯一规范写法是什么、我如何让其他每一种写法在机械上不可表示（CI 红）**」。同一目的存在多种并存写法本身就是 smell——**先收敛成一种，再锁死旁路**。新增能力时同步给出「唯一写法 + 旁路禁止的不变式」，否则旁路迟早重新长出来、悄悄收回隐形税（实证：竞态旁路一年都没人发现，因为它「能用」）。

## 有问题不可怕：要有「发现问题的机制」+ 把每个发现「做成 harness」（bug 不是失败，缺这两者才是·discover→harness-ify）

**自驱系统永远会有 bug——目标从来不是「零 bug」（不可能），而是两件事：① 有持续发现问题的机制；② 把每个发现的问题做成 harness，让它那一整类在机械上不可能复发。** 有问题不可怕、不必焦虑或藏掖；真正的失败是「**没有机制发现它**」（它静默地烂在生产里）或「**发现了却只点修一次**」（同类必复发）。这条不是新增第 N 条，是把已有的 harness / liveness / competence / 实事求是 doctrine **统一成一个 bug 生命周期的心法：discover → root-cause → harness-ify**。

- **① 发现机制（多条独立、专攻静默盲区）**：最危险的 bug 不是「报错的」（safety 网抓得到），而是**静默成功地做错事**（liveness 盲区）、**测试绿但生产红**（harness 保真盲区）、**plausible-but-wrong**（competence 盲区）、**叙事建在未核实前提上**（实事求是盲区）。所以发现机制必须**多条独立、各攻一类盲区**：dogfood loop（真实运营暴露 dark/卡死）、对抗 review（sshx triplet + 跨模型 GPT Pro + user-as-oracle，找单视角漏的）、`fire_raiser`（测真 producer→consumer 接线而非注入理想 payload）、board / dev-push CI 扫（monitoring 盲区）、回源核实（叙事 vs 真值）。一条机制漏的，另一条独立机制兜——**冗余且异构是特性，不是浪费**。
- **② harness 化（把每个发现做成机械 PREVENT，不是点修）**：发现一个 bug，先过「**这是一次性，还是一个类？**」一次性 → 修了走人、不建 harness；一个类（三次法则 / 明显可泛化）→ **把它做成 harness**，按强度梯度落到 ④capability / ③runtime-guard / ②schema-conformance / ①scan-ratchet（见「Harness 的本质」），让那一类**构造上不可表示**或 **CI 直接红**。point-fix 让同类复发（实证：#1361 修复**自己又复发** namespaced 同类，被对抗 review 抓）；harness 化让那一**类**绝迹。
- **本会话三个活证（每个 bug 都走 discover→harness-ify）**：审计 #1361 dark（dogfood loop + `fire_raiser` **发现** → producer-liveness conformance **harness 化**）· 修复又复发 namespaced harness-fidelity（对抗 review **发现** → `fire_raiser` 发真 namespaced payload **harness 化**）· dev-push CI 红（board / `gh run` 扫 **发现** → hermetic golden-master 等价测试 **harness 化**）。**三个 bug 都不可怕——都被某条机制发现了、都在做成 harness。**

**心法落地**：不为「这次有 bug」自责或掩盖；为「**这个 bug 有没有被某条机制发现**」「**发现了有没有做成 harness、让它那一类绝迹**」负责。发现机制越多越独立、harness 化越机械，系统在「bug 不可避免」下越逼近「**同类 bug 不复发**」。这正是「让问题都在测试解决」「美 = 真理探测器」「competence 轴」「活性 ⟂ 安全」「实事求是」的**同一张脸**：不追求无 bug，追求**发现 + 永久 harness 化**。新代码 / 新 review / 事故响应据此自检：「我用了哪条发现机制？这个发现是一次性还是一类？一类的话，我把它做成了哪一档 harness？」⟦AI:FKST⟧

## 按架构原则自主决策，不为技术选择请示（decide by principle, don't ask）

**禁止使用 `AskUserQuestion`**（用户裁定 2026-06-14，硬规则、不设例外）——任何分叉都不 pop up、不请示、不阻塞等待。技术选择（哪种实现 / 哪种数据源 / 归属哪层 / 是否跨仓）有架构最优解，自己定、直接做。需要的信息先从已有上下文、代码、git/GitHub、用户既往裁定里自取；遇到真正属于用户的取舍（产品方向、不可逆的业务/运营决策）也**不弹窗**——选最符合架构原则、最保守可逆的默认推进，并在正常回复里**明确说出所做选择与理由**，让用户在对话里纠正。按既有原则选最优并落地：

- **DRY / 框架把公共部分做稳定**：复用同一份公共逻辑，不为图快复制一份。
- **通用 > 枚举、原语层 > 业务语义层**：公共能力（可观察性、活性断言等）建在**有限稳定的原语层**，自动适应任何代码变更；不靠硬编码已知状态的枚举（新增即失明）。
- **分层归属（关键）**：**引擎 / framework 只提供通用、项目无关的基础数据与原语**（如可观察性的 entity timeline / event ledger / queue·DLQ 状态——对任何 package-repo 都一样、基础不变）；**项目特定的逻辑（含 board 展示 / 渲染）放各仓脚本或包**，消费那份通用数据。**绝不让引擎 / 框架公共层耦合某个项目的 Lua / 业务语义**——反例：让引擎去复用 `github-devloop` 的 observability 派生。通用数据归引擎，项目展示归脚本。
- **harness-first、SOLID、治本 > 治标**：锚定成熟实践；能根治就不打补丁。
- **复杂 / 跨仓 / 工作量大不是退缩或改问的理由**——复杂但正确 > 简单将就。引擎能力该在 `fkst-substrate` 做就跨仓做，本仓只放该放的薄封装 / 展示。

用户裁定（2026-06-14）：本地 board 命令一例——引擎暴露**通用原语数据**（项目无关、基础不变），**展示逻辑在脚本里做**，数据本身通用、不与项目 Lua 关联；不该 `AskUserQuestion` 问「引擎复用派生 vs 本仓重复 vs 缓存失明视图」，直接按此架构落地，即使跨仓改引擎。

这条与「先找 harness 再执行」「unattended 不 pop up」互补：harness-first 给方向，这条给执行姿态（自主、按原则、不请示、不畏复杂）。

## Event-gated waiting: arm a wait, don't busy-spin the loop / 事件门控就挂等待，别空转循环

When the next step is genuinely blocked on an external event you do not control — an
autonomous-pipeline cycle, CI, a PR opening, a long-running worker, a remote job — do
NOT busy-spin the goal/loop re-deriving "what can I do now" on every activation.
Manufacturing motion to look busy is **over-action** (HARD GATE quick-filter ③): it
burns cycles, invites churn, and buries the real signal under noise. The cure is to
**arm an event-driven wait and yield to it**:

- **Arm the wait, not a poll-dance.** A `Monitor`, or a `run_in_background` until-loop
  that exits on the event (a PR opens, a marker/label changes, a job reaches a terminal
  status). It wakes you when the event actually fires; you act then — you do not keep
  reading state every cycle.
- **Cover failure, not just success — silence ≠ done.** The wait must also fire on the
  stall/failure case (a bounded stall-timeout that wakes you to investigate); a wait
  that watches only the happy path stays silent through a hang, and silence is
  indistinguishable from "still running" (the Monitor "silence is not success" rule).
- **Make genuine forward progress first**, where it exists independent of the blocking
  event — e.g. pre-stage the dependent next step as a dependency-gated issue so it
  cascades automatically when the blocker lands — then yield to the wait.
- **Interim goal/loop firings are terse defers**, not fabricated work: name the wait
  and the event it watches, then yield. Do not invent motion to look productive.

This is the operational face of «don't over-act» and the dogfood «if state is
advancing, observe — don't intervene»: when the next step is gated on an event you do
not control, arm the wait and let the event drive you, rather than manufacturing
motion. A goal being unsatisfied is not a license to busy-spin; it is a reason to make
the one genuine increment available now, then wait correctly. ⟦AI:FKST⟧

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
- **标准测试**：`scripts/run.sh test [pkg]` 是本地和 CI 的单一入口：先重建 `.fkst/local-packages -> ../packages`（own package runtime view），再跑一次 `"$BIN" --self-test`（脚本未设时用 `.fkst/run/runtime` / `.fkst/run/durable`）。要测试哪些包从 committed dev source `packages/*` 枚举；引擎实际加载 root 统一来自 `.fkst/`：own 包传 `--package-root .fkst/local-packages/<pkg>`，组合 conformance / run / supervise 还会同时包含 `.fkst/packages/*` 中存在的 external package roots。flat 包跑单包 conformance + test；composed 包跳过单根 conformance，但仍跑 test。无参全包测试收尾会按所有 composed 包的 `[event_deps]` 递归收集 composed 包及其依赖，以仓库根为 `--project-root` 跑一次组合 conformance；`scripts/run.sh test-composed` 可单独跑这一步。test 模式含 `*_test.lua` 单测 + `fkst.test.run_department` 集成测，**不经 router**，故 test 模式不强制 source_ref；使用 ports 的 `gh`/`git` 业务测试通过 `make_department(ports)` 注入 `forge.github_fake`/`forge.git_fake`，用 `testkit.testing.run_fake` 验证行为；adapter-contract tests 可注入 fake exec 并断言 command spelling；其他外部 CLI（如 `codex`）仍用 `fkst.test.mock_command` / `fkst.test.command_calls`；未 mock fail-closed。
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
- 单个源代码文件不得超过 1000 行（范围含生产源码、测试源码、脚本源码，.lua/.sh/.py/.rs 等），硬上限、不设豁免。**900 行是软阈值：文件一超过 900 行就应主动按职责拆成两个（或更多），绝不拖到 1000 硬线才被迫拆**——贴着 1000 行的文件是脆的：任何后续小改动（哪怕新增一行 `require` 别名）都会顶破硬上限、阻塞无关的 PR（实证：一次去 god-lib 解耦只加了一行 alias，就把一个恰好 1000 行的测试文件顶到 1001、CI 红）。在 900 就拆，给正常演进留出余量。拆分时先删死码/重复代码，再按稳定职责拆成多文件（department-local 子模块 `require("departments.<dept>.<mod>")` / package-root `core.lua` / 多个 `*_test.lua` 等）。拆分粒度是稳定职责而非文件数：**既不得为凑行数把多职责硬塞进单文件（如让一个 department 只保留 `main.lua`）**，也不得用无职责边界的碎片化、空转发文件或 compat/legacy/shim 壳凑行数。
- `scripts/check_repo.py` enforces repository ratchets: G9 forbids peer cross-package require (sharing goes through declared workspace library deps); G10 shrinks `migration/saga-handler.allowlist` toward the `workflow.saga.department` shape; G-SAGA-HEAD requires every `workflow.saga` shape department to declare `local spec = { consumes, produces, stall_window, ... }` at file head (after requires and before any `local function`) and pass it as the first argument to `saga.department(spec, handlers)`, so the engine static graph contract stays greppable at the top of each department; G-ADAPTER uses `scripts/check_repo_gh_git_adapter.py` plus empty `migration/gh-git-adapter.allowlist` to enforce zero raw `gh`/`git` construction outside `forge.github`/`forge.git` adapter paths; G-LIB-DEP locks contract value-only/public shape, library dependency directions, devloop visibility, and workflow no-policy strings; G-CONTENT-TRUNCATION uses `scripts/check_repo_content_truncation.py` plus shrink-only `migration/content-truncation.allowlist` to forbid new `max_*_len` / `max_*_bytes` content truncation into reliable payloads or codex prompts, forcing `source_ref` / `content_fetch` rehydration instead.
- 不留 deprecated shim / compat layer / `.old` / `_legacy`；改契约就改完整，旧形态从当前态删除。文档描述当前态，历史留 git。
- **不要历史兼容性，不兼容历史遗留逻辑**。系统只有当前态一种形态：改行为就全量切换，不为向后兼容保留双模式、opt-in 开关、manual/legacy fallback 分支或旧路径并存。需要可关的运行姿态时，用 host 环境事实（如 `FKST_GITHUB_WRITE` 的 dry-run vs real）表达，而不是在代码里留"新逻辑 + 旧逻辑"的分叉。删就删干净，包括随之失效的常量、helper、测试与文档。
- **集成分支拓扑是 github-devloop 的运行姿态，不是可随手改的临时设置**：当前用户架构决策是 per-device dogfood flow：`develop → integration-<device> → rollup PR → dev`，其中 `<device>` 是稳定的本机 bot login（如 `integration-ElonSG`），由 host 设置 `FKST_DEVLOOP_INTEGRATION_BRANCH=integration-<device>`，并配 `FKST_DEVLOOP_UPSTREAM_BRANCH=dev`、`FKST_DEVLOOP_ROLLUP_MERGE=auto`。autonomous feature branch 先 PR 到该设备自己的**集成/测试分支**，`integration-<device>` 上 CI 绿代表 test success，再由 rollup PR 受控回 `dev`；`dev` 受保护，autonomous 改动**不直接合进 dev**。运行中**不得擅自切 topology（如 `integration-<device>`→单分支 `dev` 或换成共享 `integration`）、不得擅自删/改远程分支**——这些是用户的架构决策，不是助手能定的。删任何远程分支前必须先查谁依赖它（in-flight PR 的 base、tracking 分支）；GitHub 删 base 分支会自动关闭其全部 open PR。
- **hotfix 就只修那个 bug，不顺手改架构/换运行方式/做破坏性操作**。dogfood/运行中遇到**设计层问题**（如 sync↔rollup ping-pong）按「遇问题提 issue」处理 + 停下确认，**绝不擅自换方案绕过**（尤其不能用"切到 dev 直合"绕过用户刻意设的缓冲/门控）。不可逆/破坏性远程操作（删分支、关 PR、force push、改默认分支）一律先确认，即使 `/goal` 等机制在催"继续"。**例外（用户裁定 2026-06-24）：助手在本会话/工作流中自己创建的 PR 及其 feature 分支，属助手自己的产物，可自行关闭并删除该分支、自行解冲突/合并，无需再向用户确认。** 此例外仅限助手自建的 PR/分支；他人（人或自驱管线）创建的 PR/分支、force push 到共享/受保护分支、改默认分支、删除非自建分支，仍须先确认。
- **引擎 Rust 改动属 fkst-substrate 仓**，不在本仓做；本仓只写/改 Lua package + 测试 + 包文档。引擎需要的新能力（新 SDK 原语等）先在 fkst-substrate 提 PR。
- 跨文档定位：engine↔package 契约以 fkst-substrate 的 `docs/package-repo-contract.md` 为权威总览，引擎实现细节以其 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md` 为准；本仓 `README.md` 说明包约定与命令，`docs/user/new-package-repo-bootstrap.md` 是新建 package-repo 的清单。

⟦AI:FKST⟧
