# ADR 0001：std / library 依赖模型——触发点已到，采用引擎原语

- 状态：已接受（Accepted；本修订 supersede 旧的 defer / scanner-only 姿态）
- 日期：2026-06-21
- 决策方式：sshx ontology triplet 3/3 unanimous + 2 次 ChatGPT Pro 跨模型复核收敛 + 用户明确裁定「到时候了」
- 关联：`std/init.lua` 头注释（Tier S/R）、`scripts/check_repo.py` G9、`composed.deps`、fkst-website CLAUDE.md（host 仓结构 + B-std 私有 + 两边界分发）、fkst-substrate companion spec/ADR（待落地）

---

## 背景（Context）

旧版 ADR 的判断是对的：当时 `std/` 还是一个隐式 universal framework library，真实依赖可从 `require("std.x")` 静态派生；在没有强制消费者时，手写 manifest 会成为第二真相源，symlink tier-split 要么不给选择性可见，要么造成全仓 require 重写并撞 loning 的活跃重构区。因此旧决策选择：先用 scanner 作为 fitness function，保留未来上 `--lib-root` 的缝，不提前动引擎。

这个 ADR 是活文档。旧版明确列过触发点：出现选择性可见的真实需求、多命名根、第二个共享根、第三方/host 消费方够不到隐式 `std`，就应把推迟的 engine `--lib-root` / manifest primitive 拉到当前。现在触发点已经发生：

- loning 的 saga-split 把 `github-devloop` 拆成一个 family：`github-devloop`、`github-devloop-intake`、`github-devloop-decompose`、`github-devloop-pr`、`github-devloop-integration`。
- 这 5 个 package 共享的 state-machine / restart / marker / liveness / merge-gate 代码已进入 universal `std`，形态是 `std.devloop_*`。本修订时当前工作树可核实的顶层 `std/devloop*.lua` 为 42 个、11252 LOC；这些模块只被上述 5 个 devloop-family package require，其他 package 为 0。
- 这已经不是「所有包都天然可见也无害」的 `std` 小尾巴，而是第二个 library：`devloop`。它有自己的消费者集合、自己的可见性边界、自己的版本/发布语义，并且不应该让非 devloop package 获得 ambient visibility。
- 用户已明确要求依赖模型 formalized、auditable、standardized，让其他 repo / project 可以引用；per-package symlink 被裁定为隐式、不可审计、不可标准化。

因此，本 ADR 从「推迟引擎 primitive，scanner-only now」演进为「采用正式的 engine library-dependency primitive」。这不是推翻旧 ADR，而是旧 ADR 预设触发器命中后的下一阶段。

目标保持不变：依赖图必须清晰、版本边界明确、可审计、可机械验证；不制造漂移的第二真相源；不打断 loning 当前活跃迁移区；不在本仓实现引擎 Rust。

旧触发表演进为当前采用表：

| 旧版推迟项 | 旧触发点 | 当前状态 | 本修订决策 |
|--------|------|------|------|
| 手写 per-package `std.deps` manifest + `actual-uses ⊆ declared` 强制 | 引擎 `--lib-root` 命名根授权 / 第三方 public B-std 发布面 / 第二个不同信任-发布周期的共享根 | 已触发：`devloop` 是第二个共享根，且用户要求依赖图 formalized / auditable / standardized | 采用 per-unit manifest；scanner 成为 declared-vs-actual validator |
| folder / tier 物理拆分（std-core / substrate / forge） | 某模块组有独立生命周期，且有能保 require 路径不变的机制 | 部分触发：`devloop` 有 family-scoped 生命周期；但 loning 仍在 active zone | 只决定 `devloop` library 边界；实际抽取等 saga-split 稳定，且不做层级重构 |
| 引擎 `--lib-root`（命名共享根，替代 symlink） | 选择性可见的真实需求 / 多命名根 / 拆出的库 DAG 非分层 / 跨 repo 消费方够不到 std | 已触发：`std` + `devloop` 多命名根、devloop family allowlist、跨 repo 标准化需求 | 采用正式 engine library-dependency primitive |
| Tier S 经 substrate 提升发布 | 另一个 package-repo 需要 substrate 发的 Tier S 平台契约 | 尚未作为单独发布触发 | 保留为未来发布边界；不阻塞本次 library primitive |

---

## 决策（Decision）

### 1. 采用两个 unit kind：`package` 与 `library`

fkst manifest 使用中性上位词 **unit**。unit 有两个 sibling kinds：

- `package {flat|composed}` 是 runtime unit：有 departments / raisers / `M.spec` / queues / lifecycle，由 engine 加载并运行。
- `library` 是 require-only code unit：有 exports / `lib_deps` / visibility / version，没有 runtime presence，不产生 queue，不运行 department。

`library` 不是第三种 package subtype。`std` 过去已经是 library，只是被 symlink 隐藏；`devloop` 的出现让 library 从单数变成复数，概念必须显式化。

这与成熟 prior art 的方向对齐，但不是声称一一等价：Cargo、.NET、Bazel 等都支持在同一 build / manifest 世界下表达 library 与 app / binary 这类不同 target roles。fkst 采用的是同一思路：共享 manifest / lock / registry 语言，但让 `package` 与 `library` 保持不同 unit kind。

### 2. 引擎提供正式 library-dependency primitive

引擎替代 per-package `std` symlink 与 G9 no-peer-require convention，强制 require scope：

- `require()` 只能加载当前 unit 的 package-private module，或当前 unit manifest **直接声明**的 library 的 public export。
- 未声明 library access fail-closed。
- library 自身也有自己的 declared require-scope；library 可依赖 library，但必须显式声明 `lib_deps`，并由引擎解析、限权、查环。
- library access 是 **direct-only**：一个 unit 只能 `require()` 它自己直接声明的 library 的 public modules。传递 `lib_deps` 由引擎为被依赖 library 解析 / 链接，但不会自动授予上游 unit 直接 require 权；上游若要直接 require 传递库，也必须自己声明并通过 visibility。
- module ownership 由 library manifest 声明，不从目录名或 `require()` 第一段反推。
- require scope 是 capability，不是 ambient filesystem visibility；manifest 就是可审计依赖图。

这把「唯一规范写法」从 scanner convention 提升到 engine capability：业务代码不再因为 symlink 存在就天然看见所有共享代码。

### 3. Lua require scope / module cache isolation 是 engine primitive 的一部分

require-scope 不能只改路径搜索，还必须保持当前 G9 依赖的 owner-scoped module isolation：

- 每个 unit 在自己的 declared scope 内解析 `require()`；package-private modules 只属于该 package / unit，library-private modules 只属于该 library。
- library 的 public exports 只在声明该 library 的 consumer units 内可解析；private modules 永远不能被 consumer 直接解析。
- 引擎必须按 owner scope 与 requesting unit capability 给 Lua module resolution / cache keying 建模，或用等价机制达到同一性质：一个 unit 不能通过 `package.loaded` / 同名模块缓存碰撞拿到另一个 unit 的 private module，也不能拿到未声明 library 的 public module。
- 每个 unit load context 必须运行在隔离的 `package.loaded` / searcher 环境中，或 substrate 等价隔离机制中。Lua module 是经 `package.loaded` 缓存的 mutable singleton；如果只按 module name 缓存 library module，两个 consumer package 会共享同一个 mutable table，破坏 owner-scoped isolation。正确 cache key 是 **resolved module identity + content/version identity + top-level consumer/load context**。同一个 unit load context 内重复 `require()` 同一 resolved module 仍返回同一实例；不同 top-level consumer/load context 即使 require 同一 library export，也得到彼此隔离的实例。
- resolver 不得靠 searcher 顺序静默消解歧义。若两个 declared libraries export 同一 module name，或 package-private module name 与 declared library export 冲突，manifest / resolver 必须要求 canonical prefixes 消除歧义，或 fail-closed；绝不能默认选择第一个 searcher path。
- 这保留 owner-scoped module isolation，只是把 enforcement 从「没有 symlink 就看不见」提升为「没有 declared capability 就看不见」。

这是 fkst-substrate resolver 的要求；本仓不在 package 层重造 cache 隔离。

### 4. 每个 unit 一个 manifest，依赖平面分 typed sections

采用统一的 per-unit manifest（示例名：`fkst.toml`），而不是把 `composed.deps` 扩展成泛化 `deps`，也不长期分裂成两套 parallel files。

Manifest 必须区分两个依赖平面：

- `lib_deps`：code plane。声明本 unit 可以 require 哪些 library。
- `event_deps`：event plane。**只适用于 composed package，语义精确等同今天的 `composed.deps`**：声明本 composed package 的 composed graph 中被 namespaced queues 引用到的 sibling packages（按 `M.spec.consumes` / `M.spec.produces` / `M.spec.fanout`，以及实际 `raise()` 所属的 declared produced queues）。Composition conformance 据此把这些 sibling package roots 与本 package 一起加载并验证组合图。它不是泛化的「事件依赖」系统，不描述运行时投递顺序、部署依赖、版本求解、library 依赖，也不列单个 queue。

Workspace root file（示例名：`fkst.workspace.toml`）负责发现 units、配置 registries；lockfile（示例名：`fkst.lock`）负责 pin 具体版本/内容。内部 workspace 版本无关；第三方消费 named library 时 pin git ref / content id，不做 semver solving。

当前工作树里的 `composed.deps` 实例就是 `event_deps` 的迁移来源：例如 `packages/autochrono/composed.deps` 为 `consensus`，因为 `propose` produces / raises `consensus.proposal`，`reply` consumes / fanouts `consensus.consensus_reached`；`packages/github-devloop-pr/composed.deps` 为 `github-proxy consensus github-devloop-decompose`，覆盖它 graph 中的 `github-proxy.*`、`consensus.*` 与 `github-devloop-decompose.*` references。Typed manifest 只是把这些逐行文本提升为可校验字段。

Library manifest 至少声明：

```toml
kind = "library"
name = "devloop"
stable_id = "fkst.library.devloop"
version = "workspace"
lib_deps = ["std"]

[exports]
public = ["devloop.*"]

[visibility]
units = [
  "github-devloop",
  "github-devloop-intake",
  "github-devloop-decompose",
  "github-devloop-pr",
  "github-devloop-integration",
]
```

`std` 自身也成为 declared library，且 visibility 是 public：所有 units（packages 与 libraries）都可声明它。

```toml
kind = "library"
name = "std"
stable_id = "fkst.library.std"
version = "workspace"
lib_deps = []

[exports]
public = ["std.*"]

[visibility]
public = true
```

Composed package manifest 同时带两个平面，但不混用：

```toml
kind = "package"
name = "github-devloop-pr"
stable_id = "fkst.package.github-devloop-pr"
version = "workspace"
package_kind = "composed"
lib_deps = ["std", "devloop"]
event_deps = ["github-proxy", "consensus", "github-devloop-decompose"]
```

`[visibility]` 是 library 的 consumer allowlist over **units**，不是 packages-only。列表项可以是 package unit 或 library unit；`public = true` 表示所有 units 都可声明该 library。`devloop` 当前 visibility allowlist 是 devloop family package consumers；未来若另一个 library 消费 `devloop`，它应作为 unit 名加入同一 allowlist。非 devloop consumer unit 不声明它，也不能 require 它。

### 5. `fkst deps` 成为依赖审计/渲染入口

新增 `fkst deps` 命令，读取 workspace + unit manifests + lockfile，渲染完整 DAG 并检查 invariants：

- dependency graph acyclic；
- declared `lib_deps` 覆盖实际 static requires；
- 未声明 library require fail；
- visibility allowlist 被遵守；
- public exports 存在且无 owner 冲突；
- 无 orphan libraries；
- `event_deps` 与当前 `composed.deps` 语义一致：每个 composed package 声明的 sibling package set 必须覆盖它 composed graph 中 `consumes` / `produces` / `fanout` 引用到的 sibling package namespaced queues，且只用于 composed-conformance package-root inclusion，不得引入部署 / 运行时排序等更宽的 event-dependency semantics。

旧版 G-STD-DEP / requires-as-truth scanner 不删除，而是升级为 manifest validator：它继续从源码派生 actual requires，验证 declared `lib_deps` 覆盖 actual requirements。single source of truth 的含义随阶段演进：旧阶段 source 是 `require()` 本身；新阶段 source 是 manifest grant，`require()` 是被 scanner 派生出来的审计反证，二者必须一致，不能漂移。

### 6. 第一个具体 library 是 `devloop`

把 `std.devloop_*` family code 抽成 library `devloop`：

- `std.devloop_saga` 这类路径变成 `devloop.saga`；
- drop 冗余 `devloop_` prefix，避免 `devloop.devloop_saga` stutter；
- 不采用 `std-devloop`、`devloop_std` 这类保留 `std` 语义残影的名字；
- 只做机械 prefix removal + manifest declaration；
- 不在这次边界迁移里重新组织 42 个模块的层级。重新分层是语义重构，会把 boundary change 变成大范围 review/conflict 风险。

`devloop` visibility 是 consumer unit allowlist。当前 allowlist 是 devloop family packages；非 devloop consumer unit 不声明它，也不能 require 它。

### 7. 抽取前置条件：`std.devloop_prompts` 必须先反转 package-local prompt inversion

当前工作树的事实：`std/devloop_prompts.lua` 里的共享 prompt orchestration 直接 `require("prompts.implement")`、`require("prompts.fix")`、`require("prompts.sync_conflict")`、`require("prompts.review_meta")`、`require("prompts.intake")`、`require("prompts.decompose")` 等；这些 `prompts/<name>.lua` 文件位于 consuming package 自己的 package-local `prompts/` 目录。现状等价于 template-method inversion：共享 orchestration 在 `std`，具体 prompt content 由消费 package 的 package-local modules 提供。

这在 symlink-era 能工作，但如果把 `devloop_prompts` 原样移入 `devloop` library，就会变成 library → consumer package-private module 的 back-reference。新 require-scope 必须 fail-closed：library 不能看见 consumer package 的 private `prompts.*`。

因此，`devloop` 抽取前必须先 **invert the inversion by dependency injection**：

- per-package prompt content 仍由各 package 拥有；
- DI boundary 是一个正式 provider port，而不是隐式 require path。Consuming package 负责构造一个小的 `prompts` provider table，例如 `local prompts = { implement = function(ctx) ... end, fix = function(ctx) ... end, sync_conflict = function(ctx) ... end, review_meta = function(ctx) ... end, fix_reflection = function(ctx) ... end, intake = function(ctx) ... end, decompose = function(ctx) ... end }`；library entry 形如 `devloop_prompts.run(ctx, prompts)`，或等价入口但同一 port 语义。`devloop` orchestration 只调用这些 named functions / values；
- package-local prompt content 仍归 consuming package 所有；具体 template / renderer 可继续放在 consumer package 的 `prompts/` 下，由 consumer 自己 require 后塞进 provider table；
- `devloop` library 内部不得 `require("prompts.*")`；
- 禁止把每个 package 的 prompts 反过来包装成 tiny libraries 再让 `devloop` 依赖它们：那会把 package-owned content 提升成 library 依赖，反转 dependency graph，重新制造 library -> consumer-specific edge；
- `fkst deps` / G-STD-DEP validator 必须把 std/library → package-private require 视为抽取 blocker。

这是 `devloop` library extraction 的前置条件，不是后续清理项；它与现有 G-STD-DEP「`std` 不得依赖 packages」report-only 发现一致。

### 8. 引擎工作归 fkst-substrate，本仓只记录决策

Rust primitive 属于 fkst-substrate：manifest parsing、require-scope resolution/enforcement、owner-scoped module isolation 的保持、`fkst deps`、lockfile/versioning 都在 companion fkst-substrate spec/ADR 与 PR 中实现。

本仓不实现 engine，不改 Rust；本 ADR 是驱动 substrate 设计的 package-repo 决策记录。

### 9. 迁移是 staged，不是 big-bang

迁移顺序：

1. 现在：落本 ADR，并在 fkst-substrate 写 companion design/ADR。
2. 在 fkst-substrate 实现 engine primitive：manifest parsing、require-scope、`fkst deps`、lockfile。
3. 在本仓声明 `std` 为 library，给现有 packages 写 unit manifests。此阶段无行为变化，仍可由 compat path 支撑。
4. 在抽 `devloop` 前先完成 `std.devloop_prompts -> prompts.*` 的 dependency-injection inversion。
5. loning 的 saga-split 稳定后，再机械抽出 `devloop` library：保留短期 compat aliases（如 `std.devloop_*`）直到消费者迁移完。
6. engine enforcement 覆盖后，删除 per-package `std` symlink 与 G9 no-peer-require scanner 作为主防线；扫描逻辑保留为 `fkst deps` validator / ratchet。

关键纪律：现在不碰 loning 正在活跃移动的 `std.devloop_*` 文件。中途抽取会制造大量冲突，且把架构边界变更与业务迁移搅在一起。

Compat aliases 不是 enforcement bypass。它们是 scanner-tracked、removal-gated migration debt：

- aliases 必须在 `fkst deps` 维护的 shrink-only allowlist 中逐项列出 source alias、canonical target、consumer units、tracking reason；
- alias allowlist 是 canonical visibility 的 enforcement gate，不是 report-only scanner exception。若 `std` 仍是 public，且 `std.devloop_*` aliases 被当成普通 public `std` modules，任何声明 `std` 的 unit 都能绕过 `devloop` visibility 直接 reach devloop code，这是 capability bypass；
- 因此 alias resolution 必须按 canonical target grant 限权：只有 allowlist 中列出的 consumer units，且该 unit 已声明并被允许使用 canonical `devloop` library，才能解析对应 `std.devloop_*` alias；未声明 `devloop` 或不在 alias consumer allowlist 中的 unit，即使声明了 public `std`，也必须 fail-closed；
- declared-vs-actual mismatch 只有对 allowlist 中明确列出的 alias modules + consumer units 才能 warn-only；
- 未列出的 alias / undeclared library access 一律 fail-closed；
- alias 从 allowlist 删除后，对应旧路径立即按正常规则 fail-closed；
- `fkst deps` 必须有 removal gate，确保 allowlist 只能收缩到 0，不能把 `std.devloop_*` 固化成永久双入口。

### 10. 明确 supersede / preserve

**Superseded**：

- 「defer `--lib-root` / engine primitive」：触发点已命中，改为采用正式 primitive。
- 「scanner-only now」：scanner 不再是最终边界，只是 validator。
- 「universal `std` symlink 足够」：`devloop` 已证明存在第二个共享根与选择性可见需求。

**Preserved**：

- evolutionary architecture：只在触发点命中后升级机制；
- fitness functions：用机械 validator 守 require / manifest / visibility invariant；
- single source of truth：不接受 symlink 这种隐式、不可审计依赖；manifest grant 是依赖图，scanner 作反证；
- requires-as-truth 的价值：继续复用为 actual-use scanner，而不是丢弃；
- 不打断 loning active zone：抽取 `devloop` 等她的 saga-split 稳定；
- staged migration：先决策和 substrate primitive，再 manifest，再抽库，再删 symlink / G9 主防线。

---

## 理由（Rationale）

### 触发器已命中，继续 defer 变成 band-aid

旧版 ADR 的 defer 是 last responsible moment，不是永远不做。现在已经有 plural libraries：`std` 是 universal library，`devloop` 是 family-scoped library。继续把 `devloop` 放在 universal `std` 里，会让非消费者获得 ambient visibility，也让依赖图只能靠 convention 和 symlink 猜。

用户明确要求 formalized / auditable / standardized 后，symlink 的缺陷变成核心问题：它不能表达 declared capability，不能表达 visibility allowlist，不能成为跨 repo 标准接口。

### `package` 与 `library` 必须是 sibling unit kinds

把 `devloop` 做成新 package subtype 会混淆两个平面：runtime lifecycle 与 code require。`package` 会运行，有 queues 与 departments；`library` 不运行，只提供 module exports。把 library 塞进 package 类型，会让 engine、docs、audit 都被迫解释「一个不会运行的 package」，这是错误本体论。

用 unit kind 建模后，package 与 library 共享 manifest / lock / registry 语言，但各自有不同 typed sections。`lib_deps` 管代码，`event_deps` 管事件组合；这也解释了 `composed.deps` 的归宿：它不是 generic deps，而是 event plane 的正式字段。

### 引擎原语是美的解，symlink / scanner 是迁移期工具

美的解应忠于 ground truth：谁被授予 require capability，谁就能 require；谁没有声明，就 fail-closed。symlink 是 ambient filesystem visibility，scanner 是事后 detect，都不是最终边界。

这符合本仓 harness doctrine 的强度梯度：能做 capability restriction，就不长期停在 scan。`fkst deps` 仍然重要，但它的职责是审计 declared grant 与 actual require 是否一致，而不是替代 engine enforcement。

### 不制造第二真相源的方式已经改变

旧阶段没有 manifest consumer，手写 `std.deps` 会漂移，所以 `require()` 是唯一真相源。新阶段 engine 本身成为强制消费者，manifest grant 不再是重复文档，而是 capability source。此时 scanner 的正确位置是 validator：从 code 派生 actual requires，与 manifest 对账。

这保留了旧 ADR 的核心 insight：不要让两个手写叙事漂移。新模型中一个是 grant source，一个是 code-derived evidence，冲突即失败。

### 抽 `devloop` 只做边界迁移，不做语义重构

当前风险不在「名字不好看」，而在「边界不显式」。因此第一步只做机械 prefix removal 与 manifest declaration。把 42 个模块重新分层也许以后有价值，但它是独立 refactor，必须另有计划、测试与 review，不应混进 library 边界迁移。

### 跨仓归属清楚

本仓是 package repo，不改 engine Rust。真正需要的是 fkst-substrate 的 module resolver / manifest / lockfile primitive；本 ADR 只记录包库侧的决策与迁移约束，作为 companion substrate design 的输入。

---

## 后果（Consequences）

**正面**：

- 依赖图从隐式 symlink 变成 manifest DAG，可审计、可渲染、可 pin。
- `std` 与 `devloop` 的边界显式化：universal code 与 family-scoped code 不再混在同一 ambient namespace。
- `composed.deps` 获得正确归宿：它演进为 `event_deps`，不污染 code dependency plane。
- scanner 投资保留：从 scanner-only 变成 manifest validator 与 CI fitness function。
- 第三方/host repo 可以引用 named library + pinned version，而不是复制 symlink 约定。
- 引擎 enforcement 让未声明 require fail-closed，比 G9 convention 更强。

**代价 / 注意**：

- 需要 fkst-substrate 新 primitive 与 companion spec/ADR；这不是本仓 docs PR 能单独完成的实现。
- Manifest / lockfile / registry 需要设计稳定后再落地，避免把临时字段固化成 public contract。
- 抽 `devloop` 必须等 loning saga-split 稳定；现在提前搬文件会制造冲突。
- 短期 compat aliases 是迁移工具，不是永久双模式；消费者迁移完后必须删除。
- `fkst deps` 必须同时覆盖 code plane 与 event plane，否则会重新分裂出两套不可对账的依赖叙事。

**迁移完成判据**：

- 每个 unit 有 `fkst.toml`，workspace 有 `fkst.workspace.toml` 与 `fkst.lock`。
- `std` 与 `devloop` 均为 declared libraries。
- devloop family packages 声明 `lib_deps = ["std", "devloop"]`；非 devloop packages 只声明实际需要的 libraries。
- `fkst deps` 渲染 DAG 并验证 declared-vs-actual、visibility、acyclic、orphan、event_deps invariants。
- engine require-scope enforcement 生效。
- per-package `std` symlink 与 G9 作为主边界被删除；scanner 保留为 validator。

⟦AI:FKST⟧
