# FKST Local QA Host：经 NyxID 执行用户本地自动化 QA 的 MVP 实现设计

> **状态：** `local_qa_host_mvp` 当前实施设计，尚未表示对应能力已经实现。
> **命名：** 产品、进程、Profile、Schema、类型、字段和 audience 的 canonical 命名统一使用 **Local QA Host / Host**；旧草案标识不保留为兼容别名。
> **命名决策：** [ChronoAIProject/fkst-hosted#5729](https://github.com/ChronoAIProject/fkst-hosted/issues/5729)。
> **日期：** 2026-07-31。
> **系统设计：** [DESIGN.zh-CN.md](../baseline/v1/DESIGN.zh-CN.md)。
> **字段与协议规范：** [SPEC.zh-CN.md](../baseline/v1/SPEC.zh-CN.md)。
> **未来安全 Profile：** [LOCAL-QA-RUNTIME-DESIGN.zh-CN.md](../baseline/v1/LOCAL-QA-RUNTIME-DESIGN.zh-CN.md)。
> **内部拓扑语义源：** [fkst-local-qa-host-internals.mmd](fkst-local-qa-host-internals.mmd)。

---

## 1. 概述

Local QA Host 是运行在用户电脑上的受控 QA 执行宿主。Hosted 负责创建和编排 Durable QA Run，NyxID 负责把控制请求安全路由到指定设备，Local QA Host 负责本地资源与测试执行，testing packages 提供环境、Runner、Assertion 和 Evidence 领域能力，Hosted 最终完成 Artifact ingestion、Quality、Report、Publication 和 Run settlement。

一次典型运行包含以下动作：

1. Hosted 冻结 Source、Plan、Policy 和执行 Profile，并签发精确到 Run 与请求内容的业务授权。
2. Hosted 通过 NyxID 将控制请求发送到明确绑定的用户设备。
3. Local QA Host 验证本地 transport credential 与 Hosted 业务授权，幂等接受 Run。
4. Host materialize 固定 Source，启动 App、数据库和 Middleware，执行 API、CLI 或 Browser 测试。
5. Host 生成结构化测试事实，隔离并脱敏 Evidence，释放执行资源，再上传允许离开设备的 Artifact。
6. Hosted 根据固定输入完成 QualityEvaluation、确定性报告、可选发布和 RunSettlement。

Local QA Host 不能由 NyxID Node、SSH exec 或 testing-runner 单独替代。它必须长期拥有以下本地职责：

- Run admission、幂等和本地状态推进。
- Workspace、container、port、process、Chrome、Profile 和 staging 的精确所有权。
- testing packages 与宿主能力之间的 Adapter。
- 取消、超时、重启后的 Cleanup 与 reconciliation。
- Raw Evidence 的 quarantine、redaction 和 upload handoff。

Canonical 标识统一为 `local_qa_host_mvp`、`LocalQAHostService`、`LocalHostHealth`、`host_instance_id` 和 audience `fkst-local-qa-host`。物理产品根保持 `apps/local-qa-runtime/`；未来 Host 实现位于 `apps/local-qa-runtime/host/`，package 与 executable 使用 `fkst-local-qa-host`，不创建第二个 Local QA 产品根。

### 1.1 文档权威关系

| 文档 | 决定什么 |
| --- | --- |
| [SPEC.zh-CN.md](../baseline/v1/SPEC.zh-CN.md) | 跨进程字段、严格枚举、签名对象、摘要、Interface、状态机和 wire error。 |
| [DESIGN.zh-CN.md](../baseline/v1/DESIGN.zh-CN.md) | 系统级职责、信任边界、部署拓扑和完整 QA Run 生命周期。 |
| 本文 | `local_qa_host_mvp` 的内部 Module、执行顺序、最小持久化和恢复算法。 |
| [LOCAL-QA-RUNTIME-DESIGN.zh-CN.md](../baseline/v1/LOCAL-QA-RUNTIME-DESIGN.zh-CN.md) | 未来 `hardened_untrusted_code` 的 authority、隔离、fencing 和 signed recovery。 |

本文不重新定义 SPEC 中的 Schema。示例字段和表名只用于解释实现语义；若与 SPEC 冲突，以 SPEC 为准。

---

## 2. 可行性、证据与范围

### 2.1 已经验证的能力

迁移前的源仓库中唯一真实执行过的实现是 [NyxID Browser Loop PoC](https://github.com/wanghuan-520/workflow-qa/blob/docs/lightweight-local-qa-agent/poc/nyxid-browser-loop/server.mjs)。它证明了：

```text
NyxID Cloud
→ NyxID Node
→ 已人工启动的 loopback PoC service
→ playwright-core 启动系统 Google Chrome
→ 固定 fixture 点击与 DOM 断言
→ 内存 Run Result 和本机截图路径
```

因此，下列基础事实已经成立：

- NyxID 可以将请求路由到指定用户设备上的 loopback service。
- 本地服务可以启动独立的系统 Chrome 并执行浏览器动作。
- 结构化结果可以通过同一调用链返回调用方。

PoC 没有证明生产请求认证、Source materialization、Docker Compose、Readiness、通用 testing-runner、SQLite journal、取消、Artifact upload、云端报告或自动 Cleanup。

### 2.2 Target MVP 可以实现的能力

在受信任、已审查或组织明确允许的项目输入前提下，MVP 可以实现：

- Hosted 经 NyxID 控制用户本地的专用 Chrome session。
- 在 per-run Docker Compose project 中启动 App、数据库和 Middleware。
- 执行固定 StructuredPlan 中的 API、CLI 和 Browser 测试。
- 在本地隔离 raw Evidence，只上传经过脱敏与摘要绑定的内容。
- 在成功、失败、取消、超时和 Host 重启后收敛到 CleanupSummary 或明确 residual。

容器在 MVP 中只提供可重复环境和生命周期隔离，不构成 hostile-code 安全边界。

### 2.3 MVP 必须拒绝的输入

以下任一条件成立时，Host 必须拒绝 `local_qa_host_mvp`，不能静默降级执行：

- 外部 fork、未知仓库或无法信任的 dependency/lifecycle scripts。
- 开放式 Shell、Codex/Agent Action 或动态扩大文件、网络、Secret、Browser 权限。
- 生产 Secret、私网、云账号或高价值本地数据。
- 需要强 fencing、effect-before-commit recovery 或跨重启 execution takeover。
- 需要 OS 级 Browser egress enforcement、direct-socket denial 或 hostile-code 隔离。

这些场景属于未来 `hardened_untrusted_code` Profile。

### 2.4 明确非目标

MVP 不提供：

- 通用远程 Shell、任意 URL、任意 cwd/env、任意 Compose YAML 或任意 CDP endpoint。
- 对用户个人 Chrome、Profile、下载目录、SSH、Keychain、Home 或 Docker socket 的访问。
- Host 内生成最终 Quality、长期保存 Artifact/ReportRecord 或直接发布 GitHub/PQL。
- 未经 Hosted 冻结和授权的 Plan 扩权。
- Host 重启后自动从执行中断点继续 Case。

---

## 3. 系统位置、协作边界与内部组件

### 3.1 系统拓扑

```text
Hosted QA Orchestrator
    │ signed LocalQARequestAuthorization
    ▼
NyxID Cloud
    │ scoped identity / service policy / transport audit
    ▼
explicit node-pinned service
    ▼
NyxID Node
    │ local credential injection
    ▼
Local QA Host
    ├─ Ingress / Auth Adapter
    ├─ Run Coordinator + Small Journal
    ├─ Source / Workspace Manager
    ├─ Environment / Readiness Adapter
    ├─ Testing Packages Adapter
    ├─ Host Chrome Controller
    ├─ Evidence Stager / Redactor
    ├─ Artifact Upload Client
    └─ Cleanup Manager
```

控制面与 Artifact 数据面分离：

- Run 创建、查询、事件读取和取消经 NyxID 到达 Host。
- Host acceptance 快速持久化后返回，完整测试异步推进。
- 大体积 Artifact 不沿 NyxID response 返回，而由 Host 直接上传 Hosted ingestion endpoint。
- SSH exec 只用于人工批准的安装、诊断或 break-glass 运维，不是 QA Run Interface。

### 3.2 组件职责

| 组件 | 拥有 | 不拥有 |
| --- | --- | --- |
| Hosted QA Orchestrator | Durable Run、Source/Plan/Policy/Profile、设备选择、云端 Workflow、repair 编排 | 本地 container、Chrome 和资源 Cleanup |
| Hosted Authorization Authority | 精确 operation、Run、digest、device 和 TTL 的签名授权 | NyxID transport authentication |
| NyxID Cloud | 调用方身份、service/node scope、freshness、replay protection、transport audit | QA 业务授权、测试语义和 Run 事实 |
| explicit node-pinned service | service slug 到明确 Node/local endpoint 的稳定路由 | daemon、process manager、Sandbox 或测试执行 |
| NyxID Node | 出站连接、本地 credential store、credential injection、loopback 调用 | Plan 解释、资源所有权、测试判定和报告 |
| Local QA Host | 本地 admission、状态、Journal、资源、执行、Evidence、Upload、Cleanup | 最终 Quality、长期 Artifact/Report、Publication、Hosted terminal |
| testing packages | Environment、Runner、Assertion、CaseResult、Artifact 领域逻辑 | NyxID transport、设备 admission、云端 Run authority |
| Hosted Artifact/Report modules | Artifact ingestion、Quality、ReportRecord、Publication、RunSettlement | Raw local quarantine 和本地执行资源 |

### 3.3 Host 内部 Module

| Module | 责任 | 明确禁止 |
| --- | --- | --- |
| Ingress/Auth Adapter | 五个 endpoint；验证 transport credential 与 Hosted 业务授权 | 把“来自 NyxID”当作业务授权 |
| Run Coordinator | 推进 local state；协调 Prepare、Execute、Stage、Cleanup、Upload | 自行生成云端 terminal 或 Quality |
| Small Journal | 保存 request/idempotency、run、event、resource、upload/cleanup attempt | 演化为 Grant/Fence/Effect authority ledger |
| Source/Workspace Manager | 获取 digest-bound Source；创建 per-run workspace | 执行浮动 branch 或写入用户原仓库 |
| Environment/Readiness Adapter | 将冻结 Environment intent 投影为 Docker Compose 生命周期与 typed probes | 执行任意 YAML 或把单个 HTTP health 等同于全部 ready |
| Testing Packages Adapter | 调用选定 package、提供 Host capabilities、投影结果与 Receipt | 绕过 package contract 或让 Backend 自报 Pass/Fail |
| Browser Controller | 启动 Host 自有 Chrome process tree、temporary Profile 和 downloads | 附加个人 Chrome 或暴露 arbitrary CDP |
| Evidence Stager | bounded quarantine、redaction、validation、digest | 将 raw bytes 写入普通日志、event 或 cloud |
| Upload Client | post-redaction grant exchange、upload、digest reconcile | 使用未知 digest 的预签 upload grant |
| Cleanup Manager | 根据精确 ownership record 补偿清理并报告 residual | 按模糊名称、进程名或端口扫描删除资源 |

Host 是用户级部署目标，可由 LaunchAgent、桌面应用 helper 或同等用户级 supervisor 启动。它只监听 loopback 或受控 Unix socket，不要求 root LaunchDaemon。

---

## 4. 一次 QA Run 如何执行

下表是实现和排障时的主线。后续章节只展开其中某一阶段。

| 阶段 | 责任方 | 主要动作 | Durable 输出 |
| --- | --- | --- | --- |
| 1. Freeze | Hosted | 冻结 Source、Plan、Policy、Profile、device 和 `run_id` | RunSpec、StructuredPlan、Policy refs |
| 2. Authorize | Hosted | 签发绑定 method/path/body digest、Run、device、TTL 和 nonce 的业务授权 | LocalQARequestAuthorization |
| 3. Route | NyxID | Cloud 验证 transport identity；Node 注入安装级本地 credential 并调用 Host | transport audit / routing result |
| 4. Admit | Host | 验证两层授权；先查 idempotency；原子写 request、初始 state 和首个 event | acceptance、Snapshot、sequence=1 event |
| 5. Materialize | Host | 获取 exact Source；验证 digest；创建 per-run workspace | Source/workspace ownership record |
| 6. Prepare | Host + environment package | 创建 Compose project、network、volume、port；执行必要 readiness | prepared environment / readiness receipts |
| 7. Execute | Host + testing packages | 调用 runner/Backend；需要时启动专用 Chrome；生成 Observation、AssertionResult、CaseResult | structured test result refs |
| 8. Stage | Host + test-artifacts | 收集 raw Evidence；quarantine、redaction、validation、post-redaction digest | EvidenceStagingManifest / RedactionReceipt |
| 9. Cleanup execution | Host | 关闭 Chrome、runner、App/Middleware；释放 container、port、network、volume、workspace | LocalHostCleanupReceipt / residual refs |
| 10. Upload | Host + Hosted ingestion | 按对象申请短 TTL upload grant；上传或按 digest/object key 对账 | ArtifactUploadReceipt / ingest refs |
| 11. Finalize local | Host | 删除或按 TTL 保留 sanitized staging；固定本地 Outcomes | final local Snapshot / CleanupSummary |
| 12. Evaluate and settle | Hosted | ingest Artifact；冻结 ReportInputSet；Quality；Report；Publication；Settlement | ArtifactPointer、QualityEvaluation、ReportRecord、RunSettlement |

### 4.1 失败、取消和超时

失败、取消和超时不绕过主线：

1. Host 先持久化 intent 和当前已知事实。
2. 停止接纳新的 Step/effect。
3. 终止 owned Browser、runner 和 service process tree。
4. 进入 execution Cleanup，记录 released、missing、identity mismatch 和 residual。
5. 已经完成脱敏的 staging 可以继续上传或等待 bounded retry；raw quarantine 必须本地处理。
6. Hosted 根据独立 Outcomes 完成报告与 settlement，不能把 Cleanup 失败伪装为测试失败，也不能把测试失败伪装为基础设施错误。

### 4.2 Host 重启

Host 重启后：

- admission 先关闭，直到 Journal migration、resource discovery 和 health check 完成。
- `getRun` 与 cursor event read 可以恢复。
- 明确 digest/object key 的 upload attempt 可以 reconcile。
- Journal 中已知 owned resources 可以 cleanup/reconcile。
- 不自动从 `executing` 重跑或继续 Case。
- 无法确认 ownership 的资源进入 residual，不猜测删除。

---

## 5. NyxID 接入、认证与外部协议

### 5.1 四层信任

#### Hosted → NyxID

- 使用专用 workload/API identity，不复用浏览器 session。
- identity 只允许目标 Local QA service 和明确 Node 集合。
- 设置 method/path scope、rate limit 和可选 approval policy。
- 目标 Node 不可用时 fail closed，不回退到 server-side direct route。

#### NyxID Cloud → Node

NyxID 负责 Node authentication、request integrity、timestamp/nonce/replay protection 和 transport audit。FKST 依赖这些行为，但不重新定义 NyxID 内部密码学协议。

#### Node → Host

- Node 从本地 credential store 注入每个 Host 安装实例独有的 credential。
- credential 不进入 NyxID Cloud 默认 header、仓库、argv、普通日志或 Run payload。
- loopback、Unix socket、相同 UID 和“请求来自 Node”都不能替代认证。
- 生产禁止 PoC 的 `auth_method=none`。

#### Hosted 业务授权

每个非 public-health 请求必须携带 Hosted 签名的 `LocalQARequestAuthorization`，至少绑定 operation、method、canonical path、body/request digest、actor/workload、agent/device、Run、Profile、TTL、nonce、purpose 和 signature。Start variant 还绑定 Source、Plan、Policy 和 capability digest。

本地 credential 证明请求来自获准的设备 Adapter；Hosted 签名证明这个具体业务动作获得授权。两层缺一不可。

### 5.2 五个 REST endpoint

```text
GET  /v1/health
PUT  /v1/runs/{run_id}
GET  /v1/runs/{run_id}
GET  /v1/runs/{run_id}/events?after=N&limit=M
POST /v1/runs/{run_id}:cancel
```

| Endpoint | 行为 |
| --- | --- |
| Health | Public 只返回 service/version/alive；authenticated variant 可返回 admission、provider、Chrome、active runs、disk pressure 和 recovery reason。不得启动资源。 |
| Submit | Hosted 预生成 `run_id`；Host 在任何副作用前完成 authorization、strict parse 与 idempotency check。 |
| Get Run | 返回结构化 Snapshot、local Outcome、Receipt refs 和安全错误；不返回 raw Evidence、Secret 或本地绝对路径。 |
| Get Events | 返回 bounded cursor batch；允许完全相同 event 重放；同 sequence 不同 digest 是 integrity error。 |
| Cancel | 使用独立 idempotency key、digest、deadline 和 `operation=cancel` 授权；ack 只表示 intent 已接受。 |

### 5.3 幂等接受

Host 在创建 workspace、container、port 或 Chrome 前，以 `(idempotency_key, request_digest)` 查询 Journal：

- 同 key、同 digest：返回原 acceptance/snapshot。
- 同 key、不同 digest：返回 conflict，零副作用。
- 首次请求：在一个 transaction 中写 authorization digest、request、初始 state 和 sequence=1 event，再异步执行。

请求只接受 strict、digest-bound Source/Plan/Environment refs。Host 不从请求中接受通用命令执行能力。

### 5.4 Event 与断线恢复

Hosted 使用 `after` 拉取 bounded event batch，不依赖无限 SSE 或 Host 主动 push。NyxID 断线不改变 Run authority；Hosted 重连后先读取 Snapshot，再从已知 cursor 继续消费 events。

---

## 6. Source、Environment 与 Testing Packages

### 6.1 Source 与 Workspace

Host 只 materialize Hosted 冻结的 immutable Source：

- PR 使用 frozen synthetic merge object；非 PR 使用 exact commit SHA。
- provider/object/digest 必须与 SourceAcquisition 一致。
- 禁止执行时重新解析浮动 branch、tag 或默认分支。
- Workspace 位于 Host application support 下的 per-run 目录，不写入用户原仓库。

### 6.2 Docker Compose Environment

A0-A3 的具体本地 provider 是 Docker Compose。Host 根据 digest-bound execution spec：

- 创建独立 project、network、volume、ownership labels 和 loopback ports。
- 按依赖顺序启动 App、database、Middleware 和 test service。
- 只执行 Plan 当前 Case 需要的 typed readiness probes。
- 限制 CPU、memory、disk、process count 和 wall-clock。
- 禁止挂载用户 Home、SSH、Keychain、个人 Chrome、其他 repo 和 Docker socket。
- 只在 trusted-input policy 允许时执行 dependency installer 和 lifecycle scripts。

这些约束服务于可重复执行和可靠 Cleanup，不构成 hostile-code 防护承诺。

### 6.3 Testing Packages 的协作方式

Local QA Host 不重新实现 testing packages 的领域逻辑，而是提供本地能力并消费其版本化结果。

| Package/能力 | 责任 | 与 Host 的关系 |
| --- | --- | --- |
| `testing-design` | 根据 Source/需求形成候选测试设计 | Hosted 侧生成并冻结 StructuredPlan；Host 只消费已批准 Plan。 |
| `environment-factory` | Environment prepare、readiness、registration、compensation 语义 | Host 提供 Docker Compose、port、process 和 ownership Adapter。 |
| `testing-runner` | 执行 Case、计算 AssertionResult 和 CaseResult | Host 提供 argv/HTTP/Browser 等受控 Backend capabilities。 |
| Backend | 返回 CLI、HTTP、DOM、Browser 等 Observation | Backend 不得自报最终 Pass/Fail。 |
| `test-artifacts` | Artifact identity、quarantine/redaction/summary 领域逻辑 | Host 提供本地 staging、bytes IO、digest 和 upload handoff。 |
| `quality-evaluation` | 根据固定输入计算 Quality | 由 Hosted 执行，不嵌入 Host。 |
| `test-publication` | 将 ReportRecord 投影到 GitHub/PQL 等目标 | 由 Hosted 执行，不嵌入 Host。 |

Testing Packages Adapter 至少负责：

- 将 Run/Plan/Step、prepared environment、attempt 和 deadline 投影为 package invocation。
- 只暴露当前 operation 需要的 Host capability，不提供通用 Shell/FS/Network/CDP。
- 将 package event/result 规范化为 Host Journal 可持久化的 Observation、AssertionResult、CaseResult 和 safe error。
- 保存 invocation、attempt、result 与 artifact import 的可追踪关系。
- 在取消、超时和重启时终止或 reconcile package-owned child process/effect。

Package source、selected packages、contract versions、依赖闭包和 runner identity 必须在执行前锁定。精确锁结构属于 A0 contract closure，本文不发明新的 wire type。

### 6.4 Runner 判定规则

`testing-runner` 根据 Structured Assertion 计算结果：

```text
BackendObservation
→ Assertion evaluation
→ AssertionResult
→ CaseResult
→ StructuredTestResult
```

以下事实不能单独决定 Case passed：

- process `exit_code=0`
- HTTP request 成功发送
- Browser action 成功执行
- LLM/Backend 文本声称成功

缺失、截断、Schema 不兼容或互相矛盾的结果必须 fail closed 为 blocked/error/inconclusive，不能推断为 passed。

### 6.5 Host Chrome

Browser Controller：

- 使用本机安装的 Chrome executable，但每 Run 创建独立 process tree。
- 使用 temporary profile 和 isolated downloads，不接管用户现有 Chrome。
- 只执行 Plan 中的 typed BrowserAction，不暴露 arbitrary CDP。
- 默认只访问本 Run loopback target；额外 origin 必须在 Plan 中声明。
- screenshot、DOM、trace、network 和 download metadata 先进入 Evidence quarantine。
- Cancel、timeout、failure 和 restart cleanup 都必须终止 process tree 并处理 Profile/downloads。

MVP 不承诺 OS 级 direct-socket denial。需要 hostile page、强 egress enforcement 或复用个人登录状态时，必须使用独立显式授权能力或 Hardened Profile。

---

## 7. Evidence、Artifact Upload 与云端报告

### 7.1 本地 Evidence Pipeline

```text
raw observation
→ bounded quarantine
→ RedactionPolicy
→ RedactionReceipt
→ sanitized validation
→ post-redaction digest
→ EvidenceStagingManifest
→ cleanup execution resources
→ request per-object ArtifactUploadGrant
→ upload / digest reconcile
→ cleanup sanitized staging or retain until bounded TTL
```

Raw quarantine 永不进入 `getRun`、event、NyxID、Hosted report 或 object storage。Redaction 失败时不得产生可上传 Artifact。

### 7.2 先 Cleanup，再等待上传

Evidence staging 完成后，Host 先关闭 Chrome、runner、App/Middleware、container、port、network、volume 和 workspace，再等待 upload 或 Hosted 恢复。

允许暂时保留的只有：

- 已脱敏的 staging bytes。
- upload attempt 和 digest/object key。
- 必要的 Journal metadata。

这些数据必须有独立 TTL、size limit 和 cleanup outcome。云端暂时不可用不能导致用户设备长期保留运行中的服务、浏览器和端口。

### 7.3 Per-object Upload

Host 在 post-redaction digest、media type 和 size 已知后，为单个 logical Artifact 请求短 TTL、upload-only grant。Host 使用稳定 object key 和 digest 对账：上传响应丢失时查询或重试同一 logical object，不创建第二个 Artifact identity。

当前 SPEC 中历史 upload session 类型与 per-object grant flow 的一致性必须在 A0 关闭。本文采用系统设计和 Mermaid 已锁定的安全顺序，不定义新的 wire payload。

### 7.4 Hosted 报告边界

Host 输出本地执行事实：

- StructuredTestResult / CaseResult refs。
- EvidenceStagingManifest、RedactionReceipt 和 ArtifactUploadReceipt。
- LocalHostCleanupReceipt 与 profile-neutral CleanupSummary。
- execution/evidence/upload/cleanup Outcomes。

Hosted 完成：

```text
ArtifactUploadReceipt
→ ArtifactIngestReceipt + durable ArtifactPointer
→ immutable ReportInputSet
→ QualityEvaluation
→ DeterministicReport JSON / HTML / Markdown
→ optional NarrativeSupplement
→ ReportRecord + object storage
→ GitHub / PQL Publication
→ RunSettlement
```

NarrativeSupplement 可以总结失败和风险，但不能改变 Assertion、Case Pass/Fail、FinalQualityOutcome 或 publication eligibility；Report/Publication repair 不能触发本地测试重跑。

---

## 8. Local State、Journal、Cleanup 与恢复

### 8.1 Local State 与 Outcome

Local state 表示流程位置：

```text
accepted
→ preparing
→ ready
→ executing
→ staging_evidence
→ cleaning_up_execution
→ uploading
→ finalizing_local
→ terminal
```

失败、取消和超时可从任一资源持有状态进入 `cleaning_up_execution`。Local `terminal` 只表示本地必需 action 已 settled 或进入明确 repair backlog，不等同于 Hosted 已持久化 RunSettlement。

Host 独立记录：

- `execution_outcome`
- `evidence_outcome`
- `upload_outcome`
- `cleanup_outcome`

Hosted 独立记录 `report_outcome`、`final_quality_outcome` 和 `publication_outcome`。不得只用一个 status 覆盖所有维度。

### 8.2 Small SQLite Journal

Journal 是最小执行事实存储，不是安全 authority ledger。

| 逻辑表 | 最小内容 |
| --- | --- |
| `run_requests` | run/request/idempotency key、request digest、authorization digest、deadline、acceptance response |
| `runs` | local state、Outcome、source/plan/profile refs、last error、timestamps |
| `events` | run、sequence、type、payload ref、snapshot digest、created time |
| `resources` | resource id/type、provider identity、ownership label、state、cleanup action |
| `runner_attempts` | package/runner identity、invocation、attempt、deadline、result ref、outcome |
| `upload_attempts` | artifact key/digest、grant ref、object ref、attempt、outcome、retry time |
| `cleanup_attempts` | resource set digest、reason、attempt、released/residual refs、outcome |

要求：

- 使用单进程 writer 或等价串行 transaction boundary，避免同 Run 并发推进。
- request acceptance、初始 state 和首个 event 原子提交。
- 资源 create 前登记 intent，create 后写 exact provider identity；部分失败仍可 Cleanup。
- 不保存 Secret 明文、raw Artifact bytes、个人路径或 Node credential。
- migration 后未终态 Run、event cursor、resource ownership 和 attempts 仍可读。

### 8.3 Resource Ownership 与 Cleanup

Workspace、container、network、volume、port、process、Chrome、Profile、downloads、quarantine 和 sanitized staging 都必须有精确 ownership record。Cleanup 只能使用记录中的 provider identity/ownership label。

清理顺序：

1. 持久化 cancel/timeout/failure intent，停止接纳新 Step。
2. 终止 Browser、runner 和 package child process tree。
3. 停止 App、Middleware 和 test service。
4. 删除 container、network 和 volume。
5. 释放 port 和 workspace。
6. 删除 raw quarantine；sanitized staging 按 upload outcome 删除或保留至 TTL。
7. 写入 released、missing、identity mismatch、unknown 和 retryable residual。

`OwnedHandle` 与历史 `LocalResourceRecord` 的 Schema 收敛属于 A0。实现前必须选定一个规范模型，并使 Snapshot、Environment、Journal、Cancel 和 Cleanup 全部引用同一模型。

### 8.4 Restart Reconciliation

Host 启动时：

1. 关闭 admission。
2. 打开并迁移 Journal。
3. 枚举未终态 Run 和 pending attempts。
4. 根据 exact provider identity 发现 owned resources。
5. 对 upload、cleanup 和明确可重放的本地 action 做 reconcile。
6. 将无法确认 ownership 或 effect outcome 的项目标记为 residual/recovery-required。
7. Health 恢复后再开放新 admission。

MVP 不自动 resume Case，也不创建 Hardened `RecoveryDecision`、Grant、Fence 或 EffectGate。

---

## 9. 安全、凭据、审计与运维

### 9.1 凭据分离

至少区分三类凭据：

| 凭据 | 用途 | 禁止行为 |
| --- | --- | --- |
| NyxID workload/API identity | Hosted 调用目标 NyxID service/node | 复用个人浏览器 session 或扩大到通用设备操作 |
| Node → Host local credential | 证明调用来自获准 Node Adapter 和 Host installation | 进入 Cloud 默认 header、Run payload、argv、仓库或普通日志 |
| Project/test Secret refs | 精确测试动作所需 Secret | 持久化明文、写入 Event/Artifact、注入无关进程或开放式 Shell |

Secret 应通过 opaque ref 或面向精确动作的短期 material 使用。Host 只向获得批准的 process/action 注入，并确保结果、日志、截图和报告不包含 Secret canary。

### 9.2 安全不变量

- 所有非 public-health 请求同时验证 transport credential 和 Hosted business authorization。
- Profile 在 dispatch 前冻结；Host 收到 `hardened_untrusted_code` 请求必须拒绝。
- Source、Plan、Policy、Environment 和 testing package identity 在副作用前验证。
- Raw quarantine 不离开设备，不进入普通 Event。
- Host 不提供任意 Shell、任意 URL、任意 cwd/env、任意 Compose 或任意 CDP。
- Cleanup 只操作精确 owned resources，不扫描并删除“看起来相关”的宿主资源。

### 9.3 审计

- NyxID audit：transport actor、service、node、path、status。
- Hosted audit：Workflow、authorization、Policy、Quality、Report、Publication、Settlement。
- Host journal/audit：request、state、event、resource、runner attempt、upload、cleanup。

三侧使用 `run_id + request_id + request_digest + node_id + host_instance_id` 关联。NyxID transport success 不能作为 Host 已接受 Run 或本地副作用已完成的 Receipt。

### 9.4 Health、升级与卸载

- Public health 只暴露非敏感 liveness/version。
- Authenticated health 可以暴露 admission、provider、Chrome、active runs、disk pressure 和 recovery reason。
- 更新前停止新 admission；active Run 完成或进入 cancel/cleanup。
- 更新后先完成 Journal migration、resource discovery 和 health check。
- 卸载前停止新 Run，处理 active resources，并显示无法清理的 residual。

---

## 10. 复用与新增

### 10.1 复用

| 来源 | 复用能力 |
| --- | --- |
| NyxID | Cloud/Node identity、service/node route、outbound connection、credential broker、transport audit。 |
| Hosted QA | Durable Run、Source/Plan/Policy freeze、business authorization、Artifact ingestion、Quality、Report、Publication、Settlement。 |
| testing packages | testing design、environment lifecycle、runner、Assertion/CaseResult、Artifact/redaction 等领域契约与实现。 |
| System Chrome | 用户设备已安装的 Chrome executable；每 Run 使用独立 process/profile/downloads。 |
| Docker Compose | A0-A3 的 App/DB/Middleware 生命周期 provider。 |
| SQLite | Host 的最小 Run/Event/Resource/Attempt Journal。 |

### 10.2 新增

| 新增模块 | 原因 |
| --- | --- |
| Host Ingress/Auth Adapter | NyxID transport 到达不等于 QA 业务授权，需要双层验证和严格 wire mapping。 |
| Run Coordinator | 将异步本地生命周期、Outcome、Event 和 cancellation 收敛为一个可恢复流程。 |
| Source/Workspace Manager | 确保 exact Source、per-run workspace 和用户原仓库不被修改。 |
| Docker Compose Host Adapter | 将 environment contract 映射为本机 project/network/volume/port ownership。 |
| Testing Packages Adapter | 锁定 package identity，提供受控 capabilities，持久化 invocation/result/projection。 |
| Host Chrome Controller | 拥有独立 Chrome process tree、Profile、downloads 和 cleanup。 |
| Evidence Stager/Upload Client | 保证 raw quarantine、redaction、per-object grant 和 lost-ack reconcile。 |
| Cleanup Manager | 在失败、取消、超时和重启后根据 exact ownership 收敛资源。 |

### 10.3 与 Hardened Runtime 的关系

| Common concept | Local QA Host MVP | Hardened Local QA Runtime |
| --- | --- | --- |
| Local request | 五 endpoint + signed request authorization | 八方法 RuntimeService + command/fence |
| Persistence | small Run/Event/Resource/Attempt Journal | single-writer authority ledger + effect/event outbox |
| Isolation | trusted-input Docker Compose environment | per-phase VZ hostile-code Sandbox |
| Cleanup | owned resource records + compensation | sealed inventory + CleanupCapability/successor |
| Evidence | upload-only handoff | Hardened local ArtifactStore/capability read |
| Recovery | query/upload reconcile/cleanup，不自动重跑 | Hosted-signed RecoveryDecision 与 purpose-specific takeover |

MVP 不应提前实现或伪造 Hardened 对象；未来 Runtime 也不应改变 Hosted、NyxID 和 testing package 的领域所有权。

---

## 11. 实施阶段 A0-A3

### A0：契约闭合

目标是让 Hosted、Host 和 testing packages 对同一请求、结果和恢复事实达成一致，且在真实副作用前失败。

必须关闭：

- 五个 REST endpoint、method/path/header/status/error mapping。
- `LocalQARequestAuthorization` 的 canonical body/request digest、nonce、TTL 和 idempotency fixtures。
- RunSpec、StructuredPlan、Docker Compose environment 与 testing package/runner identity 的一致绑定。
- Local State、Outcome、Snapshot、Event batch 和 cursor retention。
- `OwnedHandle` 与历史 `LocalResourceRecord` 的单一资源模型。
- post-redaction per-object upload grant 与 SPEC 中历史 upload session 类型的统一。
- Journal transaction、cancel disposition、runner attempt、upload/cleanup receipt。
- Rust/TypeScript/Hosted 或实际实现语言共享的 strict-union 与 canonicalization corpus。

**Exit Gate：** 对合法、重复、冲突、过期、错误设备/Profile、未知版本/枚举和 malformed request，所有实现给出一致结果；合法请求可以原子接受且不创建执行资源。

### A1：NyxID + Host + Chrome

交付：

- 用户级 Host daemon/helper、本地 credential、双层授权。
- `health`、submit、get、events、cancel 五个 endpoint。
- Small Journal、local state/event、restart admission gate。
- System Chrome temporary profile、固定 fixture action、structured result。
- Browser/process/profile/download cleanup。

**Exit Gate：** Hosted 经明确 node-pinned service 完成一次真实 Run；Node offline fail closed；重复 submit 不重复执行；cancel/restart 后资源收敛。

### A2：Source + Docker Compose + Testing Runner

交付：

- immutable Source、per-run workspace、Compose project 和 ownership。
- App、DB、Middleware、conditional readiness。
- Testing Packages Adapter、runner、Deterministic/API/CLI/Browser Backend。
- AssertionResult/CaseResult 持久化、timeout/cancel/restart cleanup。

**Exit Gate：** 一个真实项目完成环境启动、固定和浏览器测试、结构化结果与全资源 Cleanup；不兼容 package/schema 在执行前被拒绝。

### A3：Evidence + Cloud Report + Settlement

交付：

- quarantine、redaction、sanitized validation、post-redaction digest。
- execution Cleanup 先于 grant/upload wait。
- per-object upload、lost-ack reconcile、Artifact ingestion。
- CleanupSummary、QualityEvaluation、JSON/HTML/Markdown ReportRecord、Publication/repair 和 RunSettlement。

**Exit Gate：** success、failure、cancel、timeout、Host restart 和 upload acknowledgement 丢失场景都能产生可解释 Outcome、Artifact/Report 或明确 residual，且不会重跑本地测试或泄露 raw/Secret 数据。

---

## 12. Definition of Done

### 12.1 接入与授权

- [ ] Hosted 经明确 node-pinned NyxID service 到达 authenticated Local QA Host；Node offline fail closed。
- [ ] 所有非 public-health endpoint 同时验证 local transport credential 与 Hosted business authorization。
- [ ] 同 idempotency key、同 digest 返回原结果；同 key、不同 digest 零副作用冲突。
- [ ] Host 不提供 arbitrary shell、URL、cwd/env、Compose 或 CDP endpoint。
- [ ] `hardened_untrusted_code` 请求被 Host 明确拒绝，不静默降级。

### 12.2 执行与 Testing Packages

- [ ] Source、Plan、Policy、Environment 和 testing package/runner identity 在副作用前锁定并验证。
- [ ] Host 通过 Adapter 调用 testing packages，不复制 runner/assertion/artifact 领域逻辑。
- [ ] Backend 只产生 Observation；Assertion/CaseResult 由版本化规则计算。
- [ ] API、CLI 和 Browser 至少各有一个真实成功与失败用例。
- [ ] 缺失、截断、未知版本或矛盾结果不会被推断为 passed。

### 12.3 资源、取消与恢复

- [ ] Workspace、container、network、volume、port、process、Chrome、Profile、downloads 和 staging 全部具有 Run ownership。
- [ ] success、failure、cancel、timeout 和 restart 都产生 CleanupSummary 或明确 residual。
- [ ] Host restart 不自动重跑 Case。
- [ ] Cancel intent 在停止 effect 前持久化，重复 cancel 不造成重复副作用。
- [ ] 两个 Host 实例不能同时推进同一 Run 的本地状态或资源 mutation。

### 12.4 Evidence 与 Artifact

- [ ] Raw Evidence 只存在于 bounded local quarantine，不进入普通日志、Event、NyxID 或 cloud。
- [ ] post-redaction bytes 才能申请单对象 upload grant。
- [ ] 执行资源 Cleanup 不等待云端长期可用；仅 sanitized staging 可按 TTL 保留。
- [ ] Upload 成功但 acknowledgement 丢失时，同一 object key/digest 被 reconcile，不产生第二个 logical Artifact。
- [ ] Secret canary 不出现在结果、日志、截图、Artifact、Report 或 Publication 中。

### 12.5 云端报告与终态

- [ ] Hosted 可由同一 ReportInputSet、ruleset 和 template 重放相同 digest 的确定性报告。
- [ ] Quality、Report、Publication 和 Settlement 均由 Hosted 持有，不由 Host 或 Backend 推断。
- [ ] Narrative/Report/Publication repair 不修改测试事实，也不重跑本地测试。
- [ ] execution、evidence、upload、cleanup、report、quality 和 publication Outcomes 保持独立。
- [ ] Hosted terminal 只在 RunSettlement 持久化后成立；Local terminal 不被直接当作 Hosted terminal。
