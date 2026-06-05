# CLAUDE.md

## 工作语言

源文件内部一律英文：`.lua`、`.sh`、`.py`、`.rs` 等里的注释、docstring、log/error 文本、模板字符串和标识符都保持英文，与 fkst-substrate 引擎、命令行工具和 LLM 语料一致。源文件之外的对外产物（对话回复、文档、issue/PR/comment、变更说明）用中文；代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。不要中英混杂凑句子。

## 这个仓库是什么

fkst-packages 是 fkst 的**包库**（"库 B"），承载跑在 **fkst-substrate** 引擎上的 Lua package。引擎本身在隔壁 `fkst-substrate` 仓；**本仓只写 Lua 行为层，不碰引擎 Rust**。

一个 package = `core.lua`（包内共享库）+ `departments/<dept>/main.lua`（消费/产生事件的处理器，暴露 `M.spec` 与 `pipeline(event)`）+ `raisers/<r>.lua`（cron/file_watch 触发器）+ `tests/*_test.lua`。包分两类：flat 平包必须自洽、可单根 conformance、0 外部 package namespace 引用；composed 包是一等包，负责组合/适配兄弟包，可引用 `<pkg>.<queue>`，用 `composed.deps` 声明组合 conformance 需要一起加载的兄弟包。当前 flat 包：`packages/github-proxy/`（GitHub issue/PR 入站同步 + 出站评论）和 `packages/autochrono/`（消费自有 `issue`、产出自有 `reply` 的回复起草包）。当前 composed 包：`packages/github-autochrono/`（组合 `github-proxy` + `autochrono` 的适配/wiring 包）。

## 引擎上下文（写包必须懂；权威见 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md`）

- **三级公司**：Company（supervisor + framework + composed graph）/ Department（`departments/<dept>/main.lua`）/ Person（一次 `codex exec`）。不能加层。
- **事件流** `source → fanout → route → spawn → RAISED`：raiser 静态声明 cron/file_watch；Department `M.spec` 静态声明 `consumes/produces/fanout/stall_window`。Department 收到的是 `Event{queue, payload, ts}`，**无生命周期 hook、无共享内存、无持久态**，同一 `pipeline` 跑两次是两次独立调用。
- **SDK surface（固定）**：`raise / spawn_codex_sync / spawn_codex / exec_sync / await_all / with_lock / once / cache_get / cache_set / git_log_count / git_log_grep / count_worktrees / setup_worktree / file / json.decode / log.{info,warn,error} / now`（+ test 模式 `fkst.test.{eq,is_true,is_nil,raises,run_department,mock_command,command_calls}`）。`json` 仅 `json.decode`。**包不直接碰 `<RT>`/文件系统当状态**——经原语。`once`/`cache_*`/`with_lock` 的 key 是经校验的**可读相对 path**（如 `github-proxy/issue/owner/repo/42`），不是 hex。测试中的外部 CLI 统一走 `fkst.test.mock_command` / `fkst.test.command_calls`，不生成 fake `gh` / `codex` 二进制；未 mock 的外部命令 fail-closed。
- **事实源 doctrine**：跨 pipeline 的真相只来自 git / 外部源（GitHub）/ 明确 host fact。包不在源码树或 `<RT>` 存"为活过崩溃"的业务状态；恢复靠 raiser 从源重导 + 下游按 `dedup_key` 幂等。源码树运行期只读。
- **可靠投递 / durable delivery（substrate dev 已合并）**：投递默认可靠，事件经 redb 持久 delivery（at-least-once-until-ack、lease+fencing、retry+backoff、DLQ）。对包作者：
  - **raise 到可靠下游的事件要带 `source_ref = {kind, ref}`**（稳定指针；消费者据此**回源 derive 当前真相**，不信可能过期的 payload；缺失会 fail-closed）。github-proxy 用 `{kind="external", ref="<repo>#<type>/<number>"}`（见 `core.entity_source_ref`）。
  - `M.spec.ephemeral = {"queue"}` 把某 consumed queue 退化成内存 at-most-once；`M.spec.retry = {max_attempts, base, cap}` 调重试，`retry=false` = 失败不重试（仍可靠投递）。
  - **真实 `supervise` 运行需 `FKST_DURABLE_ROOT`**（redb 落点，**不是**可清的 `FKST_RUNTIME_ROOT` scratch）；有可靠订阅却缺它会启动 fail-closed。

## 包结构约定

- **包内共享库放 package-root**：`packages/<pkg>/core.lua`，department 内 `require("core")`。**只做包内共享**——不跨包 require、不建 `fkst/` 目录、不引包间版本管理。
- **flat 包 vs composed 包**：flat 包必须自有契约、自有裸名队列、0 外部 package namespace 引用，并通过单根 conformance；composed 包可以引用兄弟包 namespace 做组合/适配，但必须放 `composed.deps` 声明所组合的兄弟包，并经组合 conformance 验证。`composed.deps` 是测试组合的最小约定，不是版本/依赖解析 manifest，也不是部署配置；这是本仓为了让组合 glue 成为 CI 覆盖的一等包而接受的取舍。
- 事件带 `schema` 字段（如 `"github-proxy.v1"`）；幂等靠 `dedup_key`（+ 出站用评论里的 HTML marker 等外部 durable 源）。
- 出站写外部（如 `gh issue comment`）会改外部状态：默认 dry-run，真写需 `FKST_GITHUB_WRITE` + 明确授权。

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
- 不留 deprecated shim / compat layer / `.old` / `_legacy`；改契约就改完整，旧形态从当前态删除。文档描述当前态，历史留 git。
- **引擎 Rust 改动属 fkst-substrate 仓**，不在本仓做；本仓只写/改 Lua package + 测试 + 包文档。引擎需要的新能力（新 SDK 原语等）先在 fkst-substrate 提 PR。
- 跨文档定位：引擎事实以 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md` 为准；本仓 `README.md` 说明包约定与命令。

⟦AI:FKST⟧
