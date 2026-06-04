# CLAUDE.md

## 工作语言

源文件内部一律英文：`.lua`、`.sh`、`.py`、`.rs` 等里的注释、docstring、log/error 文本、模板字符串和标识符都保持英文，与 fkst-substrate 引擎、命令行工具和 LLM 语料一致。源文件之外的对外产物（对话回复、文档、issue/PR/comment、变更说明）用中文；代码标识符、路径、crate/命令/协议名、测试断言、引用原文保留英文。不要中英混杂凑句子。

## 这个仓库是什么

fkst-packages 是 fkst 的**包库**（"库 B"），承载跑在 **fkst-substrate** 引擎上的 Lua package。引擎本身在隔壁 `fkst-substrate` 仓；**本仓只写 Lua 行为层，不碰引擎 Rust**。

一个 package = `core.lua`（包内共享库）+ `departments/<dept>/main.lua`（消费/产生事件的处理器，暴露 `M.spec` 与 `pipeline(event)`）+ `raisers/<r>.lua`（cron/file_watch 触发器）+ `tests/*_test.lua`。当前包：`packages/github-proxy/`（GitHub issue/PR 入站同步 + 出站评论）。

## 引擎上下文（写包必须懂；权威见 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md`）

- **三级公司**：Company（supervisor + framework + composed graph）/ Department（`departments/<dept>/main.lua`）/ Person（一次 `codex exec`）。不能加层。
- **事件流** `source → fanout → route → spawn → RAISED`：raiser 静态声明 cron/file_watch；Department `M.spec` 静态声明 `consumes/produces/fanout/stall_window`。Department 收到的是 `Event{queue, payload, ts}`，**无生命周期 hook、无共享内存、无持久态**，同一 `pipeline` 跑两次是两次独立调用。
- **SDK surface（固定）**：`raise / spawn_codex_sync / spawn_codex / exec_sync / await_all / with_lock / once / cache_get / cache_set / git_log_count / git_log_grep / count_worktrees / setup_worktree / file / json.decode / log.{info,warn,error} / now`（+ test 模式 `fkst.test.{eq,is_true,is_nil,raises,run_department}`）。`json` 仅 `json.decode`。**包不直接碰 `<RT>`/文件系统当状态**——经原语。`once`/`cache_*`/`with_lock` 的 key 是经校验的**可读相对 path**（如 `github-proxy/issue/owner/repo/42`），不是 hex。
- **事实源 doctrine**：跨 pipeline 的真相只来自 git / 外部源（GitHub）/ 明确 host fact。包不在源码树或 `<RT>` 存"为活过崩溃"的业务状态；恢复靠 raiser 从源重导 + 下游按 `dedup_key` 幂等。源码树运行期只读。
- **可靠投递 / durable delivery（substrate dev 已合并）**：投递默认可靠，事件经 redb 持久 delivery（at-least-once-until-ack、lease+fencing、retry+backoff、DLQ）。对包作者：
  - **raise 到可靠下游的事件要带 `source_ref = {kind, ref}`**（稳定指针；消费者据此**回源 derive 当前真相**，不信可能过期的 payload；缺失会 fail-closed）。github-proxy 用 `{kind="external", ref="<repo>#<type>/<number>"}`（见 `core.entity_source_ref`）。
  - `M.spec.ephemeral = {"queue"}` 把某 consumed queue 退化成内存 at-most-once；`M.spec.retry = {max_attempts, base, cap}` 调重试，`retry=false` = 失败不重试（仍可靠投递）。
  - **真实 `supervise` 运行需 `FKST_DURABLE_ROOT`**（redb 落点，**不是**可清的 `FKST_RUNTIME_ROOT` scratch）；有可靠订阅却缺它会启动 fail-closed。

## 包结构约定

- **包内共享库放 package-root**：`packages/<pkg>/core.lua`，department 内 `require("core")`。**只做包内共享**——不跨包 require、不建 `fkst/` 目录、不引包间版本管理。
- 事件带 `schema` 字段（如 `"github-proxy.v1"`）；幂等靠 `dedup_key`（+ 出站用评论里的 HTML marker 等外部 durable 源）。
- 出站写外部（如 `gh issue comment`）会改外部状态：默认 dry-run，真写需 `FKST_GITHUB_WRITE` + 明确授权。

## 构建 / 测试 / dogfood

- **引擎二进制**：本仓不含引擎。`cp env.example .env` 填 `BIN=<fkst-substrate>/target/debug/fkst-framework`。`tests/integration.sh` 按 `BIN` 覆盖 > PATH > 同级 `../fkst-substrate` 解析。
- **跑包测试**：`"$BIN" test --project-root packages/<pkg> --package-root packages/<pkg>`（test 模式：`*_test.lua` 单测 + `fkst.test.run_department` 集成测，**不经 router**，故 test 模式不强制 source_ref）。
- **dogfood / 真跑一次部门**：`"$BIN" run packages/<pkg>/departments/<dept>/main.lua --project-root <repo> --package-root packages/<pkg> --event '{"queue":"<tick>","payload":{}}'`。**坑：`run` 必须带 `--package-root`**（顶层 usage 串漏了它）。需 `FKST_RUNTIME_ROOT`（scratch）；可靠/`supervise` 还要 `FKST_DURABLE_ROOT`。`raise` 的输出是 stdout 上 `RAISED: <base64(JSON 数组)>`。
- **CI**：`.github/workflows/ci.yml` 从 `fkst-substrate@dev` 构建 fkst-framework，再跑各 `packages/*/` 的 `*_test.lua`。改包后 push `dev`/`main` 触发。

## 纪律（沿用 fkst-substrate）

- 源文件内部英文；对外中文。错误分类要窄（避免 `general error`）；日志/commit/event payload 可 grep。AI 生成的对外文本末尾保留 `⟦AI:FKST⟧`。
- 不留 deprecated shim / compat layer / `.old` / `_legacy`；改契约就改完整，旧形态从当前态删除。文档描述当前态，历史留 git。
- **引擎 Rust 改动属 fkst-substrate 仓**，不在本仓做；本仓只写/改 Lua package + 测试 + 包文档。引擎需要的新能力（新 SDK 原语等）先在 fkst-substrate 提 PR。
- 跨文档定位：引擎事实以 fkst-substrate 的 `SPEC.md` / `CLAUDE.md` / `docs/architecture.md` 为准；本仓 `README.md` 说明包约定与命令。

⟦AI:FKST⟧
