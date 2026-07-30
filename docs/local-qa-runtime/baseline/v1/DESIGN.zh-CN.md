# FKST Host 经 NyxID 触发用户本地自动化 QA：目标架构设计

> 状态：implementation-ready 设计基线，核心决策与跨边界补充已锁定，尚未实现
> 日期：2026-07-28
> 对应规范：[SPEC.zh-CN.md](SPEC.zh-CN.md)（负责字段、接口、状态机和错误等规范性细节）

## 1. 文档状态、依据与证据等级

本文解释为什么采用当前架构、各 Module 负责什么、信任边界如何建立、一次 QA Run 如何完成，以及实现过程中接受哪些权衡。本文不定义完整字段级 Schema；跨进程、跨版本和安全边界上的对象字段、枚举、Interface、摘要、签名和状态转换以 [SPEC.zh-CN.md](SPEC.zh-CN.md) 为准；Supervisor 内部 Module、事务、SQLite 和 adapter 语义以 [LOCAL-QA-RUNTIME-DESIGN.zh-CN.md](LOCAL-QA-RUNTIME-DESIGN.zh-CN.md) 为准。三份文档不得重复建立互相竞争的权威定义。

主要依据如下：

- [Mermaid 源图](fkst-host-nyxid-local-qa-flow.mmd)：架构语义基准，组件、边界、授权方向和生命周期语义以该源图为准。
- [架构图（SVG）](fkst-host-nyxid-local-qa-flow.svg)：与 Mermaid 语义同步的定制 presentation view，用于评审和展示；若标签、箭头或边界与 Mermaid 不一致，必须修正 SVG，而不是反向改变语义基准。
- [架构评审](FKST-NyxID-Local-QA-Architecture-Review.md)：给出 P0/P1 问题、职责边界和建议实施顺序。
- [Local QA Runtime 详细设计](LOCAL-QA-RUNTIME-DESIGN.zh-CN.md)：承接本文已锁定的本地实现边界、进程协议、Ledger Schema 和故障处理细节；本文仍是系统级职责与信任边界的权威来源。
- [Local QA Runtime 内部结构图（Mermaid）](fkst-local-qa-runtime-internals.mmd) 与 [SVG presentation view](fkst-local-qa-runtime-internals.svg)：表达 `apps/local-qa-runtime` 内部 host/guest/worker/browser/secret/ledger 边界；Mermaid 是内部图语义源，展示资产不得改变本文锁定的部署和授权语义。
- [NyxID 到本地 Chrome 最小闭环 POC](NyxID-Local-Chrome-Minimal-Loop-Validation.md)：提供 Cloud 到用户电脑、真实 Chrome 自动化和结构化结果回传的实证。
- [old/DESIGN.zh-CN.md](old/DESIGN.zh-CN.md)：仅用于了解历史方案的组织方式和已淘汰假设，不是当前设计依据。

本文采用三类证据等级：

| 等级 | 含义 | 本文中的例子 |
| --- | --- | --- |
| 已验证事实 | 已通过真实运行观察到 | NyxID Cloud 经 Node 到达已运行的本地 Runtime，Runtime 控制系统 Chrome 并回传结构化结果 |
| 锁定设计 | 已作出架构决策，但仍需实现和验收 | Hosted Authorization Authority 是唯一 Grant 签发者；macOS 用户级 LaunchAgent 运行 Rust Supervisor；每个 Design/Execution generation 使用独立 Virtualization.framework Linux guest |
| 待定策略 | 不影响职责边界，但实现方式仍可选择 | Artifact Store provider 的具体产品选择、离线设备等待上限、低风险 Design scope 的组织预批准范围 |

当输入材料存在冲突时，本文采用以下优先级：本轮锁定决策高于流程图中的旧标签，Mermaid 语义基准高于定制 SVG presentation view，当前流程图高于评审前方案，POC 只证明它实际覆盖的链路。`old/` 下的文档全部视为历史资料，不能作为当前实现规范。

全文统一使用以下角色和边界术语：

| 术语 | 固定含义 |
| --- | --- |
| ExternalApprovalProvider | `nyxid`、`enterprise_device_agent` 或 `local_cli` provider；使用自身受信 attestation key 签发 `DeviceAttestation`，并在 `evidence_basis="user_approval"` 路径签发对应 ApprovalEvidence。Provider 证明其声明的 assurance，不签发 FKST Grant，也不拥有 Runtime identity |
| DeviceAttestation | provider 签名的 strict object，按 `design_approval`、`execution_approval` 或 `runtime_pairing` purpose 绑定 challenge/request、用户、设备、`RuntimeIdentityStatement`、`runtime_instance_id` 与 identity/pairing epoch；它不是 Runtime 自证、Grant 或长期设备通行证 |
| Hosted Authorization Authority | `fkst-hosted` 内的 Grant 权威；验证 Policy Decision、ApprovalEvidence、DeviceAttestation 与 LocalLeaseBinding，签发 Design Grant、Execution Grant 和 Grant revocation snapshot，并仅在 `policy_not_required` 路径签发不可变 ApprovalEvidence；不签发 DeviceAttestation，不持有 Runtime identity key |
| NyxID Transport | 传输命令、Grant、事件、Receipt、Checkpoint 摘要和 Artifact 指针，不解释或扩大业务授权 |
| RuntimeIdentityStatement / RuntimePairingReceipt | Runtime 使用 device-bound non-exportable key 自签 identity statement；signed launcher 验证本地 key/binary/instance 绑定，`fkst-hosted.runtime-pairing-authority` 签发 challenge/receipt。Runtime identity 独立于 NyxID Node identity，identity 与 pairing epoch 共同约束 Grant、IPC 和撤销 delivery |
| Local Runtime Verifier | 在本地副作用发生前验证 Grant、LocalLeaseBinding、generation、fence、撤销状态与防重放信息 |
| LocalLeaseBinding | Runtime 在 Grant 签发前为指定 Run、phase、device 和候选 generation 建立的短期本地预留；它证明本机可接纳该 generation，但不是 Grant，预留本身不得推进 fence、撤销 lease 或阻断任何 active generation |
| Rust Supervisor | 由 stable signed launcher 选择并由 macOS 用户级 LaunchAgent 间接管理的长期本地监督进程；拥有 Run 验证、代际激活、单写 Ledger、VM 生命周期、EffectGate、Process Warden、Secret Broker helper 授权调用、Host Chrome Browser Provider 与 Update staging 的最终控制权，但不拥有 release-selection/rollback 指针写权，也不在自身地址空间持有 Secret 明文 |
| TypeScript Worker | 随 Runtime bundle 版本化、由 guest agent 在每个 Design/Execution Linux VM 内按任务启动或回收的 worker，承载 testing-design、testing-runner 和 Backend 编排；只能经受限 guest protocol 向 Supervisor 请求 Effect，不能直接写 Ledger 或取得宿主能力 |
| Local Policy Enforcement Point / EffectGate | Local PEP 是本地策略执行角色；v1 以 Supervisor 管理的中介式 EffectGate 实现。Design bootstrap、Execution prepare/step 与 Cleanup/repair 使用不同 strict context：Design 不要求或允许 Execution Plan 字段，Cleanup takeover 不授予执行权。Worker 或 Linux guest 只能提交类型化 PlanAction/EffectRequest，由 EffectGate 结合 phase policy、Grant、fence、inventory ownership 和本地能力执行或拒绝副作用 |
| Local Design Sandbox | 每个 Design generation 独占的 Virtualization.framework Linux guest，只承载固定源码上的受限设计活动并生成 Structured Plan，不承载正式测试执行 |
| Local QA Sandbox | 每个 Execution generation 独占的 Virtualization.framework Linux guest，承载获授权的依赖安装、服务、测试、Shell 和 Agent Action；浏览器进程仍位于宿主并经 Host Chrome Browser Provider 受控访问 |
| Single-writer Run Ledger | Rust Supervisor 独占写入的 SQLite Ledger，持久化 reservation、LocalLeaseBinding、generation/fence、effect、resource、Checkpoint、Receipt 和 cleanup/repair settlement；Worker 只经 IPC 提交事务请求 |
| Process Warden | Supervisor 内的进程与资源监护角色，统一创建、关联、停止和回收 VM、worker、Chrome、helper process、端口及其 OS 级句柄 |
| Secret Broker / CredentialLease | 与 Supervisor 同属一个签名部署目标、由 Warden 管理的独立非特权 Rust helper；它根据 EffectGate 已批准的 opaque request，向具体 Step 和冻结的 ProcessLaunchBinding 发放短期、可撤销 CredentialLease。明文只进入 broker-owned proxy、受控 guest injector 或精确目标 process domain，不进入 Supervisor、TypeScript Worker、Plan、Grant、命令正文、普通 Ledger/Event 或长期磁盘 |
| CleanupCapability | 与空 inventory lineage root 在 activation transaction 内完整建立，仅允许清理由本 Run、generation 和 Ledger 登记的对象；不依赖仍然有效的 Execution Grant。过期后只能签发同 lineage、等权或更窄 successor，不能创建新资源或取得执行权 |
| Host Chrome Browser Provider | Runtime 的宿主侧浏览器适配角色，验证 Chrome executable identity，创建专用 process、每 generation 临时 profile、typed control channel 和 deny-by-default egress proxy。只有 Browser enforcement capability probe 能证明全 Chrome process tree 的 direct TCP/UDP denial 时 Runtime 才广告该能力；否则在 reservation 前拒绝 Browser Plan，Linux guest 不直接接触个人 Chrome、任意 CDP、proxy credential、宿主 keychain 或未受控宿主网络 |
| Stable Launcher / Update Module | 两者同属 `apps/local-qa-runtime` 单一签名部署目标。Supervisor 内 Update Module 负责下载、验签、排空、migration preflight 和 staging；stable launcher 是 candidate/current/previous release selection、health confirmation 与 crash-before-health rollback 的唯一写入者。Launcher 不是第二授权权威、第二 Ledger writer 或独立部署目标 |

## 2. 背景、目标与非目标

### 2.1 背景

FKST 需要从云端创建和追踪 QA Run，同时利用用户电脑上的真实源码、设备能力和系统浏览器，在隔离的本地 QA 环境中复现项目运行条件。直接把测试放在云端 Sandbox 会失去本地设备真实性；让云端任意控制用户宿主环境又会扩大权限和审计风险。因此，系统必须同时解决四个问题：

1. 云端 Run 必须可持久化、可恢复、可取消、可审计。
2. Design 与 Execution 都必须有可验证的批准和授权；用户必须在看见实际 Structured Plan 后批准执行 scope，而不是批准某个 Grant 对象。
3. 未受信任的 PR 代码、Shell 和 Agent Action 必须限制在用户电脑上的 Local QA Sandbox Linux guest 内；需要真实 Chrome 时，只能经 EffectGate 使用宿主 Host Chrome Browser Provider。
4. 测试观察、证据、最终质量裁决和外部发布必须分层，不能让 Agent 自报结果直接成为产品结论。

### 2.2 目标

本设计的目标是建立一条完整的 Durable QA Run：

```text
Create
→ Source Acquisition
→ Design Approval
→ Reserve Design Generation
→ Design Authorization
→ Atomic Design Activation
→ Prepare Local Design Sandbox
→ Local Design
→ Design Cleanup
→ Policy Review
→ Execution Approval
→ Reserve Execution Generation
→ Execution Authorization
→ Atomic Execution Activation
→ Prepare Local QA Sandbox
→ Conditional Readiness
→ Execute
→ Evidence
→ Compensation Cleanup
→ Quality Evaluation
→ GitHub/PQL Routing
→ Finalize
```

系统应支持 PR、非 PR commit、手动入口和 API 入口；应支持 Deterministic、Browser 和 Codex Backend；应在失败、取消、超时、失联和 Runtime 重启后收敛到可解释终态。

### 2.3 非目标

本文不把以下能力归入当前设计：

- NyxID 不 Checkout 代码、不安装依赖、不启动服务、不控制浏览器、不运行测试，也不判断 Pass/Fail。
- PQL 不承担 Durable Orchestration、设备通道、本地进程管理或 GitHub 发布。
- `fkst-hosted/packages/*` 不作为独立部署目标；它们是由两个 app 组合使用的模块。
- Codex 不拥有最终测试裁决权，也不能在未经批准时扩大 Plan。
- 本文不规定完整 Schema，也不把 POC Runtime 当成生产 Runtime；v1 隔离技术已锁定为 macOS Virtualization.framework 承载 Linux guest，不再保留其他隔离技术的 v1 待定分支。
- 本文不继续采用云端 OSB QA、单个 unrestricted Codex step、Issue 评论作为唯一状态总线或顺序式 Cleanup 等历史假设。

## 3. 已验证事实与能力缺口

### 3.1 POC 已经证明的事实

2026-07-24 的最小纵向 POC 已真实跑通以下链路：

```text
NyxID Cloud
→ 用户设备上的 NyxID Node
→ 已经运行的本地 Browser QA Runtime
→ 系统 Google Chrome（playwright-core）
→ 页面访问、点击、DOM 断言和截图
→ 结构化 Run Result
→ NyxID Node
→ 调用方
```

这个结果证明：

- 用户电脑不需要开放公网入站端口；Node 通过主动建立的出站通道接收 Cloud 请求。
- Node 可以把请求转发给 loopback-only 的本地 Runtime。
- 已运行的本地 Runtime 可以在不依赖 IDE 的情况下控制真实 Google Chrome。
- 异步 `Submit + Poll` 形态可以返回 Run ID、终态、结构化结果和证据路径。
- NyxID Node 不需要执行浏览器动作或断言，它可以保持为传输和设备通道。

POC 还暴露了一个生产要求：即使临时服务声明无认证，Node 仍需要本地路由凭据条目。生产 Runtime 具备启动进程和浏览器的能力，不能沿用 `auth_method=none`；本地接口必须认证，并只监听 loopback 或受控本地 IPC。

### 3.2 POC 没有证明的能力

POC 没有证明以下生产能力：

- 用户级 LaunchAgent、Rust Supervisor、TypeScript Worker、Update Module 的安装、签名校验、注册、自启动、升级和回滚。
- 每个 Design/Execution generation 独占的 Virtualization.framework Linux guest 的创建、隔离、资源登记、清理以及两类 guest 的不可复用性。
- Local Runtime Verifier、Grant 前 LocalLeaseBinding 预留/激活协议，以及作为 Local PEP 执行机制的中介式 EffectGate 的生产实现和不可绕过性。
- DesignApprovalEvidence、ExecutionApprovalEvidence、Design Grant、Execution Grant 的签发、撤销、绑定和防重放。
- Secret Broker、最小 scope CredentialLease、lease 撤销和 Secret 不落盘语义。
- hosted lease、LocalLeaseBinding、atomic activation、generation 与 fencing 对重复执行、旧 Runtime 和延迟消息的隔离，以及预留操作不 fencing active generation 的证明。
- PR synthetic merge commit、Run 专属 SourceAcquisition 和本地源码一致性。
- Environment Factory 启动真实 App/Middleware、执行 Readiness 和可靠 Cleanup。
- Linux guest 的文件、网络、Secret、资源和进程组隔离，TypeScript Worker 的非特权边界，以及所有宿主副作用经 EffectGate 的强制路径。
- single-writer SQLite Run Ledger、Process Warden 对 VM/worker/Chrome/helper 的所有权与重启对账。
- 生产 Chrome 边界，包括宿主 Host Chrome Browser Provider、每 generation 临时 profile、宿主个人 profile/keychain 隔离和受限 control channel。
- Codex CLI 的非交互执行、Action Envelope 限制和 Plan Amendment。
- Artifact 脱敏与存储、五类 Outcome、quality-evaluation、GitHub 发布和 PQL 学习回路。
- `workflow-qa` 的端到端持久编排、重试、断线恢复、副作用去重、terminal settlement 和 terminal 后 repair。

因此，POC 是关键传输与浏览器可行性证据，不是生产实现，也不能用来降低后续授权、隔离、恢复和质量验收要求。

## 4. 核心架构原则与不变量

以下原则是实现不得破坏的不变量：

1. **云端持久编排，本地受控执行。** `apps/hosted-control-plane` 保存 Durable Run 状态；源码设计发生在 Local Design Sandbox guest，PR 代码、构建、服务、Shell 和 Agent Action 发生在独立的 Local QA Sandbox guest；真实浏览器进程位于宿主，只能经 EffectGate 和 Host Chrome Browser Provider 使用。
2. **用户批准 scope 和 Plan，不批准 Grant。** Design Approval 批准设计所需的最小 scope；Execution Approval 批准 Structured Plan、Action Envelope、Secret 用途和资源预算。Grant 是 Hosted Authorization Authority 根据可验证 Evidence 签发的机器授权对象。
3. **Plan 先于执行批准。** 系统先取得 DesignApprovalEvidence、Design LocalLeaseBinding 和 Design Grant，在原子激活后的 Local Design Sandbox 生成 Structured Plan，再由 Policy Gate 和用户审查执行范围并产生 ExecutionApprovalEvidence。
4. **签发权、设备证明、批准证明与传输权分离。** ExternalApprovalProvider 签发 DeviceAttestation，并在用户批准路径签发 ApprovalEvidence；Hosted Authorization Authority 签发两种 Grant、维护 Grant revocation snapshot，并在 `policy_not_required` 路径签发 ApprovalEvidence；NyxID Transport 只负责传输。任何一方都不能把自己的证明或通道角色扩大成其他角色的 authority。
5. **所有本地副作用必须经过 phase-specific Local PEP。** Local Runtime Verifier 先按 Design、Execution 或 Cleanup/repair strict context 判断授权对象；v1 Local PEP 由 Rust Supervisor 管理的 EffectGate 实现。Design bootstrap 不依赖 Execution Plan，Execution 才消费 Plan/Policy/envelope；`control_quiesce_reconcile` 只处理 open inventory 的 pre-seal 收敛，`control_cleanup` 只消费 sealed inventory。TypeScript Worker 和 Linux guest 只能提交类型化 PlanAction/EffectRequest，不能直接取得宿主能力。
6. **两个 Sandbox 按 generation 使用独立 Linux guest。** 每个 Design generation 与 Execution generation 都分别创建、记账、清理和销毁 Virtualization.framework Linux guest；Design Grant 不能用于正式执行，Execution Grant 不能复用旧 Design guest，新的 generation 也不能复用旧 generation 的可写 guest 状态。
7. **Plan 不可静默变化。** 执行只能发生在已批准 Action Envelope 内。新增 Step、权限、文件范围、网络目的地、Secret 或显著预算时，必须进入 Plan Amendment。
8. **Secret 只通过 Broker lease 使用。** Secret Broker 只能在批准用途与 scope 内发放短期 CredentialLease；NyxID、Hosted Authorization Authority、Backend 和 Sandbox 都不能扩大 Secret scope，明文 Secret 不进入跨边界契约。
9. **Cleanup 权限独立、可续而不扩权。** 完整 CleanupCapability 与空 inventory lineage root 在 activation transaction 中建立，不依赖有效 Execution Grant。Cleanup 前必须通过 `control_quiesce_reconcile` quiesce/reconcile并 seal 精确 inventory；owner 丢失或 capability 过期后只能通过 signed RecoveryDecision/RepairOperation、独立 `control_cleanup` FenceTransition 和同 lineage 的等权或更窄 successor 继续，不能执行测试、获取 Secret 或创建资源。
10. **Grant 前先预留，激活时才取得执行权。** Hosted 必须先取得 Runtime 基于 strict phase authorization preimage 和 AdmissionRequirements 生成的 LocalLeaseBinding，再签发绑定该证明的 Grant。首次 command admission 在一个 SQLite 事务内建立 stable environment、空 inventory root、完整 CleanupCapability、LocalExecutionLease、FenceTransition、CommandAdmissionReceipt、initial effects 和 sequence=1 outbox。预留本身不推进 fence、不撤销 active lease或创建资源。
11. **执行恢复、pre-seal 控制与 post-seal 清理必须分权。** execution takeover 必须取得新 reservation、Grant、generation/fence 和 Resume/Execute command；`control_quiesce_reconcile` 必须由 cancellation/timeout/signed RecoveryDecision 支撑，只能 suppress/quiesce/reconcile/terminate/revoke并产出 seal；`control_cleanup` 必须绑定 sealed inventory、SealReceipt 和 current/successor CleanupCapability，只能 release/delete/revoke。三种 purpose 都记录 predecessor fencing，旧 lease、命令、Receipt、worker、guest 和进程不能提交新副作用。
12. **Runner 决定测试结果。** Backend 产生观察和动作结果，`testing-runner` 用结构化断言决定 Case Pass/Fail；不采信 Codex 自报结论。
13. **质量裁决独立于测试执行和发布。** `quality-evaluation` 聚合 Case Result、Coverage 和 Evidence，分类失败并生成 Final Quality Outcome；Publication 只消费该结构化裁决，发布失败不能改变 Quality。
14. **Cleanup 是补偿阶段。** 成功、失败、取消、超时、失联和 Runtime 重启恢复都必须触发 Cleanup；Cleanup 幂等、可单独重试并输出 Receipt。
15. **Terminal 表示 settled，不表示全部成功。** 只有本 Run 所有必需 cleanup/publication action 都已成功、失败、跳过或转入明确 repair backlog 后，Run 才能 terminal；各 Outcome 可以保持失败、部分成功或 inconclusive。
16. **状态、结果与 settlement 分离。** Workflow state 表示流程位置；五类 Outcome 分别记录执行、清理、证据、发布和最终质量；action settlement 记录副作用是否已收敛，禁止压缩成单一 `Failed`。
17. **外部副作用幂等且带代际。** Prepare、Step、Cleanup、Artifact、GitHub/PQL action 都必须可重试且不重复创建。Runtime 在读取 reservation、cursor、fence 或消费 nonce 前先按 idempotency key + canonical request digest 返回既有 CommandAdmissionReceipt；Cleanup key 绑定 inventory lineage/ref/version/digest，不能把不同 snapshot 当成同一请求。
18. **契约版本化。** Plan、Grant、LocalLeaseBinding、EffectRequest、Receipt、Evidence 和 Quality 等跨边界或跨进程对象必须可验证版本与内容摘要；完整规则由 [SPEC.zh-CN.md](SPEC.zh-CN.md) 定义。
19. **核心协议不绑定 NyxID 私有接口。** NyxID Transport 是首选传输 adapter；Local QA Runtime 的核心 Run 协议保持 transport-neutral，以便未来支持本机 CLI、企业 Device Agent 或自托管通道。
20. **本地状态只有一个写入者。** Rust Supervisor 是 SQLite Run Ledger 的唯一写入者；TypeScript Worker、VM guest、Host Chrome Browser Provider helper 和 Update helper 只能通过受认证 IPC 请求事务，不能直接打开可写数据库或维护竞争性事实源。
21. **本地与 guest 通道必须双向认证并绑定启动代际。** Node/Adapter/helper 使用 method/request-digest-bound local IPC identity；Guest Agent 使用与 VM descriptor、guest boot、Runtime boot、generation/fence 和 message sequence 绑定的 boot-bound authenticated vsock session。Loopback、UID、CID 或端口本身不构成信任。
22. **宿主 Chrome 身份和实际网络连接都受控。** Browser Provider 必须验证 Chrome executable identity，并强制所有 redirect、subresource、DNS、WebSocket、service worker 和下载流量经过 deny-by-default proxy；origin allowlist 不能替代连接时 enforcement。
23. **Observation 与 Artifact 在持久化前完成安全化。** raw DOM、HTTP、日志、截图和 Trace 先进入 bounded quarantine，经过 redaction 和 sanitized validation 后才计算 publishable digest并进入普通 Ledger、Event、CaseResult、ArtifactPointer 或外部回传。
24. **升级回滚由 stable launcher 托底。** LaunchAgent 固定启动 launcher；Supervisor 只负责 staging/drain/preflight，launcher 验证 release、等待 signed health evidence并 commit 或回滚。候选 Runtime 在控制接口可用前崩溃时仍必须可恢复，rollback 后禁止自动恢复执行。
25. **Runtime identity 与 pairing 是独立、可撤销的本地身份链。** Runtime bootstrap 创建唯一 `runtime_instance_id`、device-bound non-exportable key 和 `RuntimeIdentityStatement`；initial pair、rotation、re-pair、pairing revocation 与 reset 都必须按 identity/pairing epoch fail closed。NyxID Node identity、同一用户会话或本地 endpoint 不能替代该身份链。
26. **撤销使用独立 transport control inbox。** `RuntimeService` 始终且只能有 `probeHealth`、`reserveLocalLeaseBinding`、`cancelReservation`、`submitCommand`、`getRun`、`streamEvents`、`ackEvents`、`getArtifact` 八个业务方法；`RuntimeTransportControlInbox.deliverRevocations` 独立接收 signed `RevocationBatch`，不是第九个方法，也不能承载 command、Grant、Plan 或配置。
27. **Audit 与 Ledger integrity 是 admission 前置条件。** `AuditEvent` 必须形成从 sequence=1 开始、无间隙、append-only 的签名 hash chain；`AuditCheckpoint`、`LedgerIntegrityCheckpoint` 和 `LedgerIntegrityVerificationReceipt` 必须覆盖 SQLite/WAL、audit、event outbox、effect、inventory 与 durable nonce/IPC/revocation watermarks。缺失或校验失败时普通 admission 保持关闭，禁止清空历史后恢复 healthy。

## 5. 仓库、模块与部署拓扑

目标代码结构采用 `fkst-hosted` monorepo，并把部署单元与可复用模块明确分开：

```text
ChronoAIProject/fkst-hosted
├── apps/hosted-control-plane     # 云端部署目标
├── apps/local-qa-runtime         # 用户电脑部署目标
└── packages/                     # 构建期/运行期模块，不是部署目标
    ├── qa-contracts
    ├── workflow-qa
    ├── testing-design
    ├── testing-runner
    ├── backend-contract
    ├── environment-factory
    ├── test-artifacts
    ├── quality-evaluation
    └── test-publication

ChronoAIProject/NyxID             # 独立产品与设备通道
YueZh127/product-quality-loop     # 独立 PQL 测试资产系统
```

### 5.1 独立部署目标

| 部署目标 | 运行位置 | 主要目的 |
| --- | --- | --- |
| `apps/hosted-control-plane` | FKST 云端 | Durable Run、调度、Checkpoint、Policy Gate 协调、Hosted Authorization Authority、结果裁决与外部 adapter |
| `apps/local-qa-runtime` | 用户电脑 | 用户级 LaunchAgent、Rust Supervisor、TypeScript Worker、Local Runtime Verifier、LocalLeaseBinding、EffectGate、Virtualization.framework Linux guest、single-writer SQLite Run Ledger、Process Warden、Secret Broker、Host Chrome Browser Provider、Update Module、Receipt 和 Artifact 指针回传 |
| ExternalApprovalProvider / NyxID Cloud/Node | provider-defined；云端与用户电脑 | ExternalApprovalProvider 签发 DeviceAttestation/用户批准 Evidence；NyxID Cloud/Node 提供 NyxID provider 实现、NyxID Transport、凭据来源 adapter 与审计 |
| PQL | 独立服务/仓库 | 版本化测试策略、Project Pack、用例资产和学习闭环 |

`apps/hosted-control-plane` 与 `apps/local-qa-runtime` 必须独立构建、版本化、发布和升级。它们共享 packages 和契约，但不能被打包成必须同步部署的单一进程。`apps/local-qa-runtime` 对用户只形成一个签名、安装、升级和回滚的部署目标；其 LaunchAgent、Rust Supervisor、TypeScript Worker、VM guest helper、EffectGate、Process Warden、Secret Broker、Host Chrome Browser Provider、Ledger 和 Update Module 是同一 app 的内部角色或随包组件，不得拆成需要独立安装、独立版本协商或独立授权根的部署目标。Hosted Authorization Authority 仍只属于 hosted app。

### 5.2 Packages 的约束

`packages/*` 是可组合模块，不是服务或 Daemon。它们不得被文档、CI 或部署系统描述为独立部署目标。testing packages 不得依赖 `apps/*` 的实现；app 负责提供数据库、队列、网络、文件系统、浏览器、凭据和日志等具体适配。

这种结构保留 monorepo 的原子变更和契约一致性，同时阻止“共享代码等于共享部署”的耦合。

## 6. 组件职责与边界

### 6.1 两个 app

| 组件 | 负责 | 不负责 |
| --- | --- | --- |
| `apps/hosted-control-plane` | Run 持久状态、Durable Orchestration、RunDraft/SourceAcquisition/RunSpec、调度、Checkpoint、hosted lease/generation/fence、取消与恢复、Policy Gate 协调、Hosted Authorization Authority、`fkst-hosted.runtime-pairing-authority` / `fkst-hosted.revocation-authority` modules、NyxID Transport adapter、Quality/Settlement、GitHub/PQL adapter | 直接执行 PR 代码、Shell、浏览器动作、本地进程管理，签发 ExternalApprovalProvider 的 DeviceAttestation，持有 Runtime identity key，或把 Policy Gate 当作本地副作用执行点 |
| `apps/local-qa-runtime` | 以用户级 LaunchAgent 启动 stable launcher 和 versioned Rust Supervisor；管理 device-bound Runtime identity、pairing epoch 本地状态、strict reservation/atomic activation、独立 revocation control inbox、single-writer Ledger、append-only Audit/checkpoint、phase EffectGate、VZ guest、Warden、process-bound Secret、verified/proxied Chrome、quarantine/redaction、recovery/Cleanup 与 staged update/rollback | 签发 Grant、DeviceAttestation 或 hosted pairing receipt，替用户批准、扩大 Plan/Secret、决定 Final Quality、直接发布 GitHub，或让 launcher/Worker/guest 成为第二授权权威 |

### 6.2 app 内部角色与 adapter

| 角色或 adapter | 所属 app | 责任边界 |
| --- | --- | --- |
| Hosted Authorization Authority | `apps/hosted-control-plane` | 验证 Policy Decision、ApprovalEvidence、DeviceAttestation 与 LocalLeaseBinding，作为唯一 Grant 签发者签发 Design Grant 和 Execution Grant，维护 Grant revocation snapshot，并在 `policy_not_required` 路径签发不可变 ApprovalEvidence；不签发 provider DeviceAttestation、不持有 Runtime identity key，也不把 LocalLeaseBinding 当作授权 |
| Rust Supervisor / Local Runtime Verifier | `apps/local-qa-runtime` | 由 launcher 启动；持有 device-bound Runtime identity key 的受控 handle 与 epoch 状态，按 Design/Execution/cleanup strict context 验证签名、authorization preimage、binding、generation/fence、TTL、nonce 和撤销状态，先处理 command idempotency，再原子创建 environment/inventory/capability/lease/outbox；不签发 Grant、DeviceAttestation 或 pairing receipt，也不选择 release |
| TypeScript Worker | `apps/local-qa-runtime` | 在每阶段 Linux guest 内承载 testing-design、testing-runner 和 Backend 协调；经 guest agent/vsock 读取最小授权快照、提交 EffectRequest 与 observation，不承载 NyxID Transport adapter，不直接持有宿主权限或可写数据库 |
| Local Policy Enforcement Point / EffectGate | `apps/local-qa-runtime` | 对 Design bootstrap、Execution prepare/step、Cleanup/repair 使用不同授权公式；执行 typed PlanAction/EffectRequest，拒绝 context 混用、越 envelope、旧 fence、未 seal inventory 或 control-cleanup authority 创建资源，不签发或扩大 Grant |
| VM Manager | `apps/local-qa-runtime` | 使用 Virtualization.framework 为每个 Design/Execution generation 创建独立 Linux guest，提供最小镜像、受控 source 输入与 guest agent channel；不复用跨 generation 可写磁盘 |
| RuntimeTransportControlInbox | `apps/local-qa-runtime` | 独立于八方法 RuntimeService，只接受 signed exact-object `RevocationBatch`；在同一事务更新 Grant/Artifact access watermark、阻断后续 effect/read、写 audit outbox并持久化 signed `RevocationDeliveryReceipt`。Ack 只证明 durable apply，不表示 Cancel 或 Cleanup 完成 |
| Single-writer Run Ledger | `apps/local-qa-runtime` | 由 Supervisor 独占写入 SQLite，保存 reservation、binding、lease/fence、effect/resource、Checkpoint、Receipt、cleanup/repair settlement、revocation watermark 与更新状态；其他角色只经 IPC 请求事务 |
| Audit / Ledger Integrity | `apps/local-qa-runtime` | 维护无间隙 append-only `AuditEvent` hash chain，生成 `AuditCheckpoint` 与 clean transaction boundary 上的 `LedgerIntegrityCheckpoint`，由独立 verifier 产生 `LedgerIntegrityVerificationReceipt`；未通过时关闭普通 admission并保留只读诊断和最小 cleanup recovery |
| Process Warden | `apps/local-qa-runtime` | 为 VM、worker、Chrome、helper、proxy 和端口建立 ExecutableIdentity、ProcessLaunchBinding 与 OS handle ownership，执行 quiesce、kill、reap、残留扫描和 seal 前 reconcile；不按名称/PID/路径模糊清理 |
| Secret Broker helper | `apps/local-qa-runtime` | 作为 Warden 管理的独立非特权 Rust helper，使用 Supervisor 已授权的 opaque request 换取短期 CredentialLease，并通过 broker-owned proxy、受控 guest injector 或精确 process domain 绑定 ProcessLaunchBinding、Step、destination、generation/fence；不签 Grant、不解释 Plan、不写 Ledger，也不把 Secret 暴露给 Hosted、NyxID、Supervisor、TypeScript Worker、普通 Ledger/Event 或持久 disk |
| Host Chrome Browser Provider | `apps/local-qa-runtime` | 验证 Chrome executable identity，创建专用 process/profile、typed BrowserAction channel 和 forced egress proxy；只有 enforcement capability probe 能证明整个 Chrome process tree 的 direct TCP/UDP denial 时才广告 Browser capability，经 EffectGate 执行动作并在持久化前交给 Redaction Gate；不开放个人状态、任意 CDP 或 direct network bypass |
| Stable Launcher | `apps/local-qa-runtime` | 固定由 LaunchAgent 启动，验证 release selection、路径、签名、security epoch 和 compatibility，启动 candidate并等待 signed health evidence，commit 或回滚 previous；不处理 Run/Grant/Effect，不形成第二部署目标 |
| Update Module | `apps/local-qa-runtime` | 校验 update manifest/migration window，排空 reservation，stage compatibility set并请求 launcher activation；不直接写 candidate/current/previous selection，也不能在 rollback 后自动 resume execution |

这些角色可以在 app 内拆成 Rust module、TypeScript Worker、guest agent 或受控 helper process，但必须随 `apps/local-qa-runtime` 作为一个部署目标安装和升级。进程拆分不能引入第二个本地授权权威、第二个可写 Ledger、独立常驻 Daemon，或允许 Worker/guest 绕过 Supervisor、EffectGate 和 Process Warden。

### 6.3 Testing packages

| Module | 负责 | 不负责 |
| --- | --- | --- |
| `qa-contracts` | 跨 app、NyxID Adapter、PQL Adapter 的公共契约与兼容规则 | 持久化、传输或业务编排 |
| `workflow-qa` | 完整 QA Run 的 workflow 定义、阶段依赖、补偿关系和 effect 语义 | 作为独立进程运行或直接管理本地进程 |
| `testing-design` | 根据源码、项目配置和 PQL 资产生成 Structured Plan | 批准 Plan、签发 Grant 或执行测试 |
| `testing-runner` | 选择 Backend、推进 Step、执行结构化断言并决定 Case Pass/Fail | 擅自扩大 Action Envelope 或生成最终发布策略 |
| `backend-contract` | 统一 Deterministic、Browser、Codex Backend 的执行与取消边界 | 规定具体浏览器、Agent 或 Shell 实现 |
| `environment-factory` | Prepare、条件 Readiness、资源登记和补偿 Cleanup 的接口与领域逻辑 | 作为独立部署服务；具体宿主能力由 local app 注入 |
| `test-artifacts` | Case Result、EvidenceManifest、脱敏、摘要、访问范围和保留策略 | 最终质量裁决或 GitHub 路由 |
| `quality-evaluation` | 聚合结果、Coverage、Evidence，分类失败并生成 Final Quality Outcome 与稳定去重依据 | 执行测试、修改 PQL 资产或直接调用 GitHub |
| `test-publication` | 根据 QualityEvaluation 生成幂等 Publication Plan | 自行重新解释原始日志或绕过 QualityEvaluation |

### 6.4 NyxID 与 PQL

`ExternalApprovalProvider` 是 `nyxid`、`enterprise_device_agent` 或 `local_cli` 的 strict provider union。Provider 使用自己的受信 attestation key 签发 `DeviceAttestation`，并在用户批准路径签发对应 ApprovalEvidence；NyxID 只是其中一个 provider/adapter，不是 DeviceAttestation 的唯一可能来源。Hosted Authorization Authority 只在 `policy_not_required` 路径签发 ApprovalEvidence，并作为唯一 Grant 签发者验证 provider Evidence、DeviceAttestation 与 LocalLeaseBinding。Runtime 自己建立 device-bound identity key 和 `RuntimeIdentityStatement`，`fkst-hosted.runtime-pairing-authority` 签发 `RuntimePairingChallenge`/`RuntimePairingReceipt`；禁止复用 NyxID Node identity 作为 Runtime identity。

NyxID Transport 负责安全反向连接以及 reservation request、LocalLeaseBinding、命令、Grant、`RevocationBatch`、事件、Receipt 与 Checkpoint 传输；凭据来源 adapter 可参与 Secret Broker 的 lease 获取。NyxID 不拥有 FKST Run 状态，不创建本地 reservation 或 LocalLeaseBinding，不签发任何 FKST Grant，不修改 Grant 或 Plan，不扩大 CredentialLease 的 Secret scope，不执行测试，也不判断测试质量。

PQL 负责 Product Map、Test Catalog、Project Pack、Regression Suite、Fixture、Selector、Scope Policy、Coverage Gap 和 Asset Change Proposal。它向 `testing-design` 提供版本化输入，并消费经质量裁决后的测试资产反馈；它不调度设备、不管理 Sandbox，也不发布产品仓库状态。

## 7. 信任模型与双阶段授权

系统跨越云端、ExternalApprovalProvider、设备传输通道、宿主 Supervisor、TypeScript Worker、Linux guest、宿主 Chrome 和不受信任代码，不能把“用户点了批准”或“组织策略允许”简化成一个可转发布尔值。Provider-signed DeviceAttestation、ApprovalEvidence、Hosted Authorization Authority 签发的 Grant、RuntimeIdentityStatement/RuntimePairingReceipt 与 LocalLeaseBinding 分别证明不同事实，禁止互相替代。Policy Gate 负责云端策略判断，Local PEP 通过宿主 EffectGate 负责本地逐动作强制执行；两者处于不同边界，缺一不可。Grant 签发前还必须由目标 Runtime 产生 LocalLeaseBinding，以证明候选 generation 在正确设备上有未激活的本地预留。信任链包含以下角色：

| 角色 | 信任职责 |
| --- | --- |
| Policy Gate | 分别检查 Design scope 和 Execution Plan 请求的命令、文件、网络、Secret、资源和目标设备，产生可审计 Policy Decision |
| ExternalApprovalProvider | 用受信 attestation key 对 `DeviceAttestation` 签名；对需要用户批准的路径，证明哪个用户在什么设备上下文中批准了哪个 scope/Plan，并签发对应 ApprovalEvidence。其 assurance 只能按声明解释，不能把 `software_bound` 冒充硬件证明 |
| Hosted Authorization Authority | 验证 Policy Decision、ApprovalEvidence、DeviceAttestation 与目标 Runtime 的 LocalLeaseBinding，作为唯一 Grant 权威签发 Design Grant 和 Execution Grant，维护 Grant revocation snapshot、nonce、TTL、audience、generation、fence 与审计关联；不签发 provider DeviceAttestation或持有 Runtime identity key |
| NyxID Transport | 将 pairing/attestation challenge、reservation request、LocalLeaseBinding、不可篡改 Grant、`RevocationBatch` 和 Run 命令送达目标端点，并持续回传事件、Receipt、Checkpoint 摘要和 Artifact 指针；不解释 binding、Grant 或 revocation fact |
| Local Runtime Verifier | 只信任当前 active Runtime identity/pairing epoch 与配置的 provider/hosted trust roots；在 Grant 签发前建立未激活 reservation并返回 LocalLeaseBinding，Grant 到达后在单写 Ledger 事务中验证签发者、设备、RunSpec、source revision、Plan 摘要、scope、binding、有效期、generation、fence、撤销与重放状态并原子激活 |
| Local Policy Enforcement Point / EffectGate | 对已经通过验证的具体 EffectRequest 再次实施宿主文件、网络出口、进程、Secret、browser control 和资源限制；guest 内部隔离由 Linux/VM 边界补充，但不能替代 EffectGate 对宿主能力的中介 |

### 7.1 Runtime identity bootstrap、pairing、rotation、re-pair、revoke 与 reset

安装后的 signed launcher 必须先验证 launcher/Supervisor `ExecutableIdentity`、Keychain/Secure Enclave key handle 与本地 installation binding，再由 Runtime 创建唯一 `runtime_instance_id`、`installation_id`、device-bound non-exportable signing key 和自签 `RuntimeIdentityStatement`。首次 identity 固定 `identity_epoch=1`，本地 pairing state 从 `pairing_epoch=0` 开始；Runtime identity 不复用 NyxID Node identity，也不由 ExternalApprovalProvider 或 Hosted Authorization Authority 持有。

Initial pairing 使用 `fkst-hosted.runtime-pairing-authority` 签名、短 TTL 的 `RuntimePairingChallenge(purpose="initial_pair")`。Runtime 用当前 identity key 签完整 challenge；ExternalApprovalProvider 产生 `DeviceAttestation(purpose="runtime_pairing")`，绑定同一 challenge/request/user/device/identity epoch并明确 pairing receipt 尚不存在。`fkst-hosted.runtime-pairing-authority` 验证后签发 active `RuntimePairingReceipt`。只有未过期、未撤销且 identity/pairing epoch 与 Runtime 当前状态完全一致的 Receipt，才可绑定后续 DeviceAttestation、Grant、LocalIPCBinding 与 revocation delivery。

Identity key rotation 保持 `runtime_instance_id`、递增 `identity_epoch`、绑定 previous statement，并要求旧 key 与新 key 分别证明 continuity；rotation 完成后必须强制 re-pair。旧 key compromise 无法证明 continuity 时禁止伪造 rotation，必须走 emergency reset。普通 re-pair 保持 identity epoch、严格递增 `pairing_epoch`，并原子 retire 旧 LocalIPC session、旧 pending reservation 和尚未消费的旧 pairing-bound Grant。

Emergency pairing revoke 包括用户 unpair、设备移除、identity rotation、key compromise、Runtime reset 或 provider security action。撤销必须立即关闭 reservation、command、Artifact read 与 transport-control ack path，使旧 epoch 的 DeviceAttestation、Grant、reservation 和 IPC session fail closed；已经持久化的本地 CleanupCapability 仍可在不依赖旧 pairing 在线有效性的前提下收敛已登记资源。Emergency reset 必须销毁旧私钥、endpoint/session material，撤销旧 pairing，分配新的 `runtime_instance_id` 与 `installation_id`，以 `identity_epoch=1`、`pairing_epoch=0` 重新 bootstrap；禁止沿用旧 Grant、Ledger authority、nonce/revocation watermark、IPC sequence 或 attestation。

### 7.2 Design Approval Evidence 与 Design Grant

Design 阶段必须先产生 `DesignApprovalEvidence`。Evidence 绑定用户或预批准策略主体、目标设备、RunDraft/SourceAcquisition、允许读取的 source revision 与目录、PQL 资产、网络和资源上限、有效期及 nonce。需要用户批准时由 ExternalApprovalProvider 产生可验证 Evidence；若 PolicyDecision 明确 `approval_requirement.kind="not_required"`，则由 Hosted Authorization Authority 生成并签名 `evidence_basis="policy_not_required"` 的不可变 Evidence。两条路径都不能用空值或“默认允许”绕过审计链。

在 Design Grant 签发前，Hosted 先经 NyxID Transport 请求目标 Runtime 为候选 Design generation 建立 reservation。Rust Supervisor 在 single-writer SQLite Ledger 中检查设备身份、Runtime 版本、可用容量、RunSpec 摘要和 phase 冲突，写入短期未激活 reservation，并返回签名或设备认证绑定的 `LocalLeaseBinding`。该 binding 不是 Grant，也不授予 Worker、guest、Secret 或浏览器能力；创建、续短或过期 reservation 都不得推进当前 accepted fence、撤销 active lease、停止 active guest，或以任何方式 fencing 现有 active generation。

Hosted Authorization Authority 同时验证 Design Policy Decision、`DesignApprovalEvidence` 与 `LocalLeaseBinding`，随后作为唯一签发者签发绑定该 reservation 摘要的 Design Grant。Grant 到达 Runtime 后，Supervisor 必须在一个 Ledger 事务中验证 Grant 与 reservation 精确匹配、尚未过期且未被消费，再把 reservation 原子转换为 active local lease/generation；失败时不得创建 guest 或产生副作用。Design Grant 只允许在新的 Local Design Sandbox 中获得生成 Plan 所需的最小能力，例如读取固定 source revision、静态分析、读取批准范围内的项目元数据和 PQL 资产。它不允许启动被测服务、执行正式测试、访问长期 Secret、扩大网络访问、写入被测仓库或复用 Local QA Sandbox。

### 7.3 Execution Approval Evidence 与 Execution Grant

Structured Plan 生成并完成 Design Cleanup 后，Policy Gate 对 Plan、Action Envelope、Secret 用途、资源预算和目标设备产生 Execution Policy Decision。ExternalApprovalProvider 收集用户对该 Plan 的批准并生成 `ExecutionApprovalEvidence`；Evidence 必须绑定 `run_id`、`effective_sha`、`plan_digest`、`policy_digest`、设备、批准 scope、generation 候选和有效期。

在 Execution Grant 签发前，Hosted 再请求目标 Runtime 为候选 Execution generation 建立独立 reservation，并取得绑定 `run_id`、phase、device、RunSpec/Plan 摘要、候选 generation、Runtime capability digest、nonce 与短 TTL 的 `LocalLeaseBinding`。Execution reservation 与已经清理的 Design generation 不共享 guest、可写磁盘或 lease；如果旧 generation 仍 active，Runtime 可以拒绝新 reservation，或仅保留不激活预留，但绝不能通过预留提前提高 fence 或终止旧 generation。

Hosted Authorization Authority 验证 Execution Policy Decision、`ExecutionApprovalEvidence` 与 `LocalLeaseBinding`，再作为唯一签发者签发绑定本次 RunSpec、固定 source revision、Plan、目标设备、reservation、generation、fence 和批准范围的 Execution Grant。用户批准的是 Plan 与 scope，Execution Grant 是 Authority 据此签发的机器授权结果。Runtime 收到 Grant 后，以单个 Ledger 事务消费 reservation 并原子激活 local lease；只有激活成功后才可创建 Execution guest、启动 Worker Step 或发放 CredentialLease。

NyxID Transport 只传输 reservation 协议消息、LocalLeaseBinding 和两种 Grant，不能制造或解释 binding，不能重写 scope、延长 TTL、换绑设备、reservation 或 generation，也不能扩大 Secret Broker 最终发放的 CredentialLease。Local Runtime 只信任 Hosted Authorization Authority 的 Grant 签名、与本地 reservation 精确匹配的 binding 和已配置的信任根，不因为请求来自 NyxID 通道就自动执行。

这种分离避免把设备通道变成业务授权中心：NyxID 可以替换或扩展，FKST 的授权语义、撤销规则、fencing 和审计责任仍由 FKST 控制。

## 8. Source Revision 与 PR 默认语义

测试结论必须对应一个固定、可重放的代码状态。源对象按 `RunDraft → SourceAcquisition → RunSpec` 演进：RunDraft 只记录触发输入和候选设备；SourceAcquisition 解析或生成不可变 Git 对象并记录获取方式、内容摘要与保留信息；只有 SourceAcquisition 完成后才冻结 RunSpec，作为 Approval Evidence、Grant、Plan、generation、Evidence 和 Quality 的共同绑定对象。RunSpec 冻结后不得继续跟随可变分支名。

### 8.1 PR Run

PR 默认测试固定的 synthetic merge commit：

1. RunDraft 只记录 repo identity、PR number、触发信息、profile、policy 和候选设备；它不提前声明精确 base/head SHA，也不是可授权执行规范。
2. Hosted Source Resolver 在 SourceAcquisition 中解析精确 base/head SHA，使用这两个对象创建 synthetic merge commit，把不可变 Git 对象保存在 Runtime 可获取的受控 ref 或 bundle 中，并记录内容摘要与 effective SHA。
3. 系统用 SourceAcquisition 结果冻结 RunSpec；Local Design Sandbox 只获取并校验该 effective SHA，不在设备端重新生成 merge commit。
4. 后续 DesignApprovalEvidence、ExecutionApprovalEvidence、LocalLeaseBinding、Design Grant、Execution Grant、Plan、Evidence、QualityEvaluation 和 Publication 全部绑定该 RunSpec 与 effective SHA；同一 Run 的合法幂等重试可以复用仍有效对象，换 generation 的恢复必须取得新的 LocalLeaseBinding/Grant，但不重新按最新 base 生成 source revision。

选择 synthetic merge commit，是为了默认验证“PR 合入目标分支后的集成结果”，同时避免依赖可能变化或过期的临时 merge ref。代价是需要明确处理 merge conflict、对象保留和生成元数据；如果无法创建 merge commit，Run 应进入可解释的 Blocked 状态，不得静默退回仅测试 head SHA。

### 8.2 非 PR Run

非 PR Run 使用 exact commit SHA。分支名、标签或 Issue 中的文本只能用于解析入口，不能成为执行期 source identity。

### 8.3 变更后的重新运行

PR head、base 或任何 effective revision 变化时，必须创建新的 `run_id`，重新执行 SourceAcquisition、冻结新 RunSpec，并重新完成 Design Approval/Authorization。禁止在原 Run 内创建 revision attempt，也禁止通过 Plan Amendment 改变 `effective_sha`；revision 变化会使旧 DesignApprovalEvidence、ExecutionApprovalEvidence、LocalLeaseBinding、Design Grant、Execution Grant、Plan 绑定和所有待提交 Evidence 全部失效，因为用户批准的是旧代码、旧 scope 与旧 Plan 的组合。

## 9. 完整 QA Run 生命周期

一次正常 Run 按以下阶段推进：

| 阶段 | 核心行为 | 主要责任方 |
| --- | --- | --- |
| Create | 创建 RunDraft，记录 repo identity、PR number 或 revision hint、触发信息、profile、policy 和候选设备；精确 SHA 由 SourceAcquisition 解析 | hosted-control-plane |
| Source Acquisition | 生成或解析固定 effective SHA，持久化 synthetic merge 对象、受控 ref/bundle、摘要和保留信息 | hosted-control-plane |
| Freeze RunSpec | 用 SourceAcquisition 结果冻结 RunSpec，建立后续 Evidence、Grant、Plan 和 generation 的共同绑定 | hosted-control-plane |
| Design Approval | Policy Gate 检查最小 Design scope；需要用户批准时由 ExternalApprovalProvider 产生 DeviceAttestation 与 Evidence，`policy_not_required` 时由 Hosted Authorization Authority 产生不可变 Evidence | hosted-control-plane + ExternalApprovalProvider |
| Reserve Design Generation | Hosted 请求候选 Design generation；Rust Supervisor 在单写 Ledger 中建立未激活 reservation 并返回 LocalLeaseBinding。预留不推进 fence，也不影响任何 active generation | local-qa-runtime + hosted-control-plane + NyxID Transport |
| Design Authorization | Hosted Authorization Authority 验证 Evidence、Policy Decision 与 LocalLeaseBinding，作为唯一签发者签发绑定 reservation 的 Design Grant | hosted-control-plane |
| Activate Design Generation | Local Runtime Verifier 验证 Grant 与 reservation，在单个 Ledger 事务中消费 reservation、激活 local lease/generation；失败时不创建资源 | local-qa-runtime |
| Prepare Local Design Sandbox | EffectGate 使用不依赖 Plan/PreparedEnvironment 的 Design bootstrap context 创建独立 VZ guest；Warden 登记 VM/session/worker，EffectGate 只开放 Source/DesignPolicy/DesignScope | local-qa-runtime |
| Local Design | TypeScript Worker 在 guest 中获取并校验 effective SHA，读取批准的 PQL 资产，经 EffectGate 请求宿主能力，生成 Plan v1 与摘要 | local-qa-runtime + testing-design |
| Design Cleanup | Supervisor 关闭 Design ordinary EffectGate，quiesce/reconcile/seal inventory，使用 Design generation 的 CleanupCapability 回收 Worker/guest/channel/staging 并输出 seal/termination/Cleanup Receipt | local-qa-runtime + environment-factory |
| Policy Review | Policy Gate 检查 Plan 的 Action Envelope、资源、网络、browser control 与 Secret 请求 | hosted-control-plane / Policy Gate |
| Execution Approval | ExternalApprovalProvider 收集针对 Plan/scope 的用户批准与设备证明，产生 DeviceAttestation 与 ExecutionApprovalEvidence | ExternalApprovalProvider |
| Reserve Execution Generation | Hosted 请求候选 Execution generation；Supervisor 建立独立未激活 reservation 并返回绑定 Plan/capability 的 LocalLeaseBinding，不复用 Design guest 或 lease，也不 fencing active generation | local-qa-runtime + hosted-control-plane + NyxID Transport |
| Execution Authorization | Hosted Authorization Authority 验证 Evidence、Policy Decision 与 LocalLeaseBinding，作为唯一签发者签发绑定 reservation 的 Execution Grant | hosted-control-plane |
| Dispatch and Activate | Runtime 先按幂等键/请求摘要返回既有 Receipt；首次 admission 在单个事务原子创建 environment、空 inventory root、完整 CleanupCapability、lease/FenceTransition、CommandAdmissionReceipt、initial effects 和 outbox | hosted-control-plane + NyxID + local-qa-runtime |
| Prepare Local QA Sandbox | VM Manager 为该 Execution generation 创建新的 Virtualization.framework Linux guest；Process Warden 登记 guest/worker/端口/helper，EffectGate 创建 Environment 所需的受控副作用 | local-qa-runtime + environment-factory |
| Readiness | 只执行 Plan 声明的条件检查；API/CLI/单元测试不强制启动 Host Chrome Browser Provider | environment-factory |
| Execute | TypeScript runner Worker 选择 Backend；guest 内运行不受信任代码，宿主副作用以 EffectRequest 经 EffectGate 执行，并用结构化断言决定 Case Pass/Fail | testing-runner + local-qa-runtime |
| Evidence | raw observation/bytes 进入 bounded quarantine，完成 redaction/sanitized validation 后生成 Case Result、EvidenceManifest 和 post-redaction Artifact Pointer | test-artifacts |
| Cleanup | Supervisor 先 quiesce/suppress/reconcile并 seal inventory，再用 exact snapshot/ref/version/digest、InventorySealReceipt 和 current/successor CleanupCapability 回收资源；takeover 只允许 cleanup actions，不依赖旧 Grant | environment-factory + local-qa-runtime |
| Return（全程） | 持续回传带 generation/fence 的 Event、Receipt、Checkpoint、Outcome，以及经 quarantine/redaction 后的 sanitized observation 和 Artifact Pointer；raw output 禁止进入普通回传流 | local-qa-runtime + NyxID |
| Quality | 聚合 Case、Coverage、Evidence，分类失败并生成固定 source revision 的最终质量结论 | quality-evaluation |
| Publish | 幂等发布 GitHub Check/Comment/Summary，并按规则创建 Issue 或 PQL Proposal；每个 action 独立 settlement | test-publication + hosted adapters |
| Finalize | 对账所有 command/effect、Receipt、Checkpoint、cleanup/publication action 和 repair backlog；全部 settled 后关闭 Durable Run | workflow-qa / hosted-control-plane |

### 9.1 拒绝、取消与 Policy Block

用户拒绝或 Policy Gate 拒绝时，Run 进入 Blocked，并记录原因和审计事件。Design Approval 被拒绝时尚未建立 Design reservation；Execution Approval 被拒绝时必须确认 Design Cleanup 已 settled。Authority 在 reservation 后拒绝签发 Grant、Grant 过期或 Dispatch 放弃时，Supervisor 只需在 Ledger 中取消或等待未激活 reservation 过期；reservation 不应拥有 guest、worker、Secret、Chrome 或其他需 CleanupCapability 清理的执行资源。若拒绝发生在 atomic activation 或任何资源创建之后，Runtime 必须用 CleanupCapability 清理该 generation 已登记资源。

取消请求由 hosted-control-plane 持久化。对未激活 reservation，必须有幂等 reservation cancellation/supersession 记录，使晚到 Grant command 无法激活，但不改变 active fence。对 active generation，Authority 撤销 Grant并签发 cancellation/transition authority，再发送 Cancel command。Grant/Artifact access 撤销事实不混入 `submitCommand`：hosted 通过 NyxID Transport 把 signed `RevocationBatch` 投递到独立 `RuntimeTransportControlInbox.deliverRevocations`。Runtime 对 batch chain、freshness、nonce、sequence、previous digest 与 watermark 做 strict 校验，并在同一 Ledger 事务 durable apply watermark、阻断后续 Grant effect/Artifact read、写 audit outbox和持久化 signed `RevocationDeliveryReceipt`；ack 只表示该 batch 已 durable apply，不表示取消或 Cleanup 已完成。

Supervisor 在一个 admission transaction 中持久化 cancel intent、抑制未 dispatch 的 ordinary effect，并进入 `control_quiesce_reconcile`：只允许 quiesce、termination、revoke 和非破坏性 reconcile。只有 inventory barrier 已证明 pre-seal effect 收敛并原子生成 sealed inventory 后，才可激活 `control_cleanup` lease/capability successor并执行 release/delete。两种 purpose 都不能创建资源或取得 execution authority，晚到 completion 只用于对账。

### 9.2 失联、lease 与恢复

Hosted control plane 持有每个 active generation 的 hosted lease；Rust Supervisor 在 single-writer SQLite Run Ledger 中持有由 LocalLeaseBinding 原子激活得到的 local lease、最近接受的 fence、Checkpoint、effect/resource ownership 和 Process Warden 句柄。TypeScript Worker、guest agent 与 Host Chrome Browser Provider helper 不保存权威 lease 状态。网络失联不能让 Hosted 假定本地已经停止，也不能让 Runtime 无限持有执行权。active lease 到期后 EffectGate 必须拒绝新的非清理副作用，Supervisor quiesce Worker/guest，并保留 CleanupCapability 完成收敛。

撤销 freshness 也是本地 authorization 的必要条件。Runtime 必须持续检查 Grant/Capability 声明的 `max_snapshot_age_seconds` 与 `max_delivery_age_seconds`、当前 `RevocationBatch` hash chain和 durable watermark；freshness 超限、batch gap、previous digest 不匹配、watermark rollback 或 pairing 被撤销时，必须关闭相应 reservation/command/effect 或 Artifact read path，进入 snapshot/re-delivery 请求与只读诊断。连接中断、Node 重放或 Runtime restart 后，相同 batch 只能返回原 durable ack；禁止把“尚未收到撤销”或“ack 丢失”解释为继续授权。

恢复不能直接续用旧 lease，也不能把 reservation 当作 fence。Runtime 先关闭 admission、建立 local recovery latch、失效旧 session，只做只读 discovery并上报 Snapshot/Checkpoint/effect/inventory high-water mark；Hosted 随后签发与这些事实、old fence、cursor、TTL/nonce绑定的 RecoveryDecision。`resume` Decision 必须先绑定已取得的新 Execution reservation/Grant，再由 recovery ResumeCommand原子激活 execution takeover；`reconcile_and_seal` 使用 `control_quiesce_reconcile`，seal后由独立 `replay_cleanup` Decision使用 `control_cleanup` 和 capability successor；`advance_from_receipt` 只消费 Decision 列出的权威 Receipt。不同 purpose禁止互换。

Supervisor 重启后以 SQLite WAL/事务记录为权威，保持全局 recovery latch 和 EffectGate ordinary path closed，失效旧 IPC/vsock session，并使用 Warden 对账 VM、worker、Chrome/executable identity、proxy、端口、materialization 和 quarantine。Hosted 决定前只做非破坏性 reconcile；任何 Worker 自报状态都不能覆盖 Ledger。Update/rollback 也进入同一 Snapshot、takeover、reconcile、seal、Cleanup 或 Resume 协议。

CleanupCapability 不因普通 Grant 或 active lease 到期而失效，但自身过期后不能静默接受。只要已登记危险资源仍未 settled，cleanup authority 可以基于 signed RecoveryDecision/RepairOperation 签发同 lineage、等权或更窄 successor。Cleanup 前必须证明 pre-barrier effect 已收敛并 seal 精确 inventory；Cleanup 不能创建新 guest/worker、CredentialLease、Chrome 或测试，late resource discovery 必须产生新 inventory descendant、seal 和 attempt。

## 10. Local QA Runtime 与 Local QA Sandbox

### 10.1 单一部署目标与 v1 进程模型

`apps/local-qa-runtime` v1 是 macOS-first 的单一签名部署目标。安装包注册当前用户 LaunchAgent，固定启动一个最小 stable signed launcher；launcher 验证 release selection、路径 containment、Apple/FKST 签名、security epoch 与 compatibility，再启动 versioned Rust Supervisor。v1 不使用系统 LaunchDaemon；launcher 只拥有狭窄 boot/release authority，不是第二 Grant authority、EffectGate 或 Ledger writer。

Rust Supervisor 是本机 QA 控制面、NyxID Transport adapter 宿主、Runtime device-bound identity/epoch 本地状态持有者和唯一 SQLite writer。TypeScript Worker 只在 Design/Execution Linux guest 内启动，不持有 lease/fence、Runtime identity key、宿主能力或可写数据库。Guest agent、Chrome helper 和 update helper 通过双向认证并绑定 executable/boot/generation 的 protocol 受 Supervisor 管理。Update Module 可以 stage release 和请求 activation，但只有 launcher 能 commit current/previous selection并在候选 crash-before-health 时回滚。所有角色仍随同一个 `apps/local-qa-runtime` 部署目标签名、安装和升级。

v1 必须覆盖：

- 用户级 LaunchAgent、stable launcher、登录启动、candidate health confirmation、crash-before-health rollback、卸载和用户可见状态。
- Rust Supervisor、TypeScript Worker、guest/helper/Chrome 的签名/ExecutableIdentity、版本兼容、LocalIPCBinding 与 boot-bound authenticated vsock 双向认证。
- NyxID Node Adapter 的 replay-protected loopback/IPC 路由；外部 transport request 不能直接到达 Worker 或 guest。
- `RuntimeService` 只暴露 `probeHealth`、`reserveLocalLeaseBinding`、`cancelReservation`、`submitCommand`、`getRun`、`streamEvents`、`ackEvents`、`getArtifact` 八个业务方法；请求/响应使用 epoch-bound、双向 durable sequence/previous-digest/nonce authentication chain。
- 独立 `RuntimeTransportControlInbox.deliverRevocations` 只接收 signed `RevocationBatch` 并返回 durable `RevocationDeliveryReceipt`，不得成为第九个 RuntimeService 方法或通用 control/config channel。
- Runtime identity bootstrap、initial pairing、identity key rotation continuity、强制 re-pair、普通 re-pair、pairing revoke和 emergency reset；旧 identity/pairing/session epoch 的 Grant、attestation、reservation、IPC 与 revocation ack全部 fail closed。
- single-writer SQLite Ledger、CommandAdmissionReceipt、WAL/事务恢复、lease/FenceTransition、canonical effect、inventory seal、Checkpoint/Receipt 和 settlement 对账。
- append-only signed `AuditEvent` chain、`AuditCheckpoint`、`LedgerIntegrityCheckpoint` 与独立 `LedgerIntegrityVerificationReceipt`；只有覆盖当前 durable high watermark 的 passed verification 才允许开放 ordinary admission。
- 完整 VZ boot-chain manifest 校验，以及每个 Design/Execution generation 的 guest 创建和销毁。
- Warden 对 VM、guest agent、Chrome、proxy、helper、端口、ProcessLaunchBinding、materialization 和 quarantine 建立所有权并完成取消/恢复/Cleanup 对账。
- Update Module 的 staged update/drain/migration preflight，以及 launcher 的原子 selection、health gate、anti-rollback 和失败回滚。
- Runtime recovering/admission/capacity/disk/outbox、capability、release、schema/guest compatibility 报告，使 reservation/Grant 绑定实际可执行能力。

本地 application support 权限、签名 release、ExecutableIdentity、LocalIPCBinding、hash chain 与 integrity checkpoint 对同一 UID 攻击面的承诺是**篡改检测与 fail-closed**：检测 binary、Ledger、session、sequence、nonce 或 checkpoint 被替换/回滚后关闭 admission并保留诊断。它不承诺防止同一 UID 主体终止 Runtime、删除不可保护的数据、耗尽磁盘/CPU 或阻断网络，因此不是 DoS prevention 或宿主可用性隔离边界；恢复可用性依赖受保护备份、重新安装/reset、hosted 对账与明确的运维响应，不能把“进程仍可被 kill”误报为完整性控制失效。

Linux 和 Windows Runtime 属于后续阶段。公共 contracts 和 backend interfaces 不应写死 macOS，但 v1 验收、发布和安全基线明确以用户级 LaunchAgent、Rust Supervisor 与 Virtualization.framework 为准。

### 10.2 Host、Worker、Guest、Browser、Secret 与 Ledger 边界

Runtime 在宿主系统上运行并负责控制，但不受信任代码、Node/TypeScript orchestration 和宿主能力不能合并为一个无限权限进程。本地实现必须保留以下可独立审计的边界：

| 边界 | 承载内容 | 强制约束 |
| --- | --- | --- |
| Host boot/Supervisor boundary | stable launcher；Rust Supervisor、Verifier、reservation/activation、EffectGate、VM/Warden、Update staging | launcher 只选择/回滚签名 release；Supervisor 是 local lease/fence 与 Ledger 唯一权威。二者都不运行 PR 脚本，且互不取得对方的 boot 或 QA authorization 权限 |
| TypeScript Worker boundary | Design/Execution guest 内的 testing-design、testing-runner、Backend 协调和协议序列化 | 非特权、随 VM 回收；只经 boot-bound authenticated vsock 提交 typed PlanAction/EffectRequest；不能直接访问 Ledger、宿主文件、Secret material、Chrome/CDP、proxy credential 或 VZ handle |
| Design guest boundary | 每个 Design generation 独立的 Virtualization.framework Linux guest、固定 source 输入、静态分析、批准的项目元数据与 PQL 资产、Structured Plan 生成 | 仅在 Design Grant 原子激活后创建；不得启动正式测试环境；不与 Execution 或其他 generation 共享可写磁盘、内存、agent channel 或进程 |
| Execution guest boundary | 每个 Execution generation 独立的 Virtualization.framework Linux guest、依赖安装、被测 App/Middleware、测试进程、Shell 与 Agent Action | 仅在 Execution Grant 原子激活后创建；不受信任代码留在 guest；宿主副作用必须经 EffectGate；不得复用 Design guest 或旧 generation 的可写状态 |
| Host browser boundary | verified Chrome executable、专用 process/profile、typed control channel、forced proxy、enforcement probe 和 quarantine capture | guest/Worker 只能经 EffectGate 请求 BrowserAction；只有 probe 证明全 process tree 的 IPv4/IPv6 TCP/UDP 均被强制中介时才广告 Browser capability。enforcement 丢失必须先阻断网络再终止 Session；不能附加其他 Chrome、访问 CDP/个人状态、绕过 proxy 或把 raw observation直接持久化 |
| Host secret boundary | 独立非特权 Secret Broker helper、CredentialLease、ProcessLaunchBinding、process injector/proxy 和 materialization audit | Supervisor 只处理 opaque ref、授权绑定和 receipt；明文不进入 Hosted、NyxID、Supervisor、TypeScript Worker、Plan、Grant、普通 Ledger/Event、Artifact 或持久 disk。proxy mode 下仅 broker 持有明文；guest injection 下受控 injector 与获准 process domain 是临时 custodian；env/file mode 必须显式声明获准 descendants、继承和擦除范围 |
| Ledger boundary | Supervisor 独占写入的 SQLite 数据库与受保护的本地密钥材料 | reservation、binding、activation、fence、effect/resource、Checkpoint、Receipt 和 settlement 在事务中更新；Worker/guest/helper 只读派生快照或经 IPC 请求写入，不能建立竞争性事实源 |

本文对 `Workspace`、`Sandbox` 和 `Environment` 作严格区分：Workspace 只指某个固定 source revision 的文件视图或 checkout；Sandbox 是一个 generation 独占的 Linux guest 隔离边界；Environment 是 Execution guest 内为测试创建的 App、Middleware、端口和数据库等运行资源集合。宿主 Chrome 是 Environment 可使用但不位于 guest 内的受控资源。三者不能作为同义词，也不能用“创建 Workspace”代替创建 Sandbox 或准备 Environment。

### 10.3 EffectGate 与 Sandbox 强制路径

Local PEP 是架构角色，EffectGate 是 v1 的具体执行机制。Runtime coordinator、TypeScript Worker、guest agent 或 Browser Backend 只提交 strict PlanAction/EffectRequest。Design bootstrap context 绑定 Source/DesignPolicy/DesignScope，不携带 Execution Plan；Execution prepare/step 绑定 Plan/Policy/envelope；Cleanup/repair 绑定 sealed inventory、capability 和 takeover purpose。EffectGate 先校验 active lease/FenceTransition、CommandAdmissionReceipt 与 context，再执行或返回既有 Receipt；任何 caller 都不能取得可脱离 Gate 使用的 allow token。

EffectGate 负责中介的宿主能力至少包括：source/bundle 导入、受控文件导出、网络出口代理、宿主进程与端口、CredentialLease、Host Chrome browser control、Artifact 导出以及 Cleanup/repair。guest 内部普通文件和进程活动由 Linux 权限、只读/临时磁盘、网络设备配置和 VM 资源上限约束；guest 不获得宿主目录直通挂载、宿主 Docker socket、任意 loopback、任意 USB、个人 keychain 或通用 hypervisor control。仅有 VM 隔离不能替代 EffectGate，EffectGate 也不能声称观察 guest 内每个 syscall；两者共同构成边界。

M2 的“首个完整 untrusted-flow Gate”不是只证明 VM 能启动。Design/Execution 任一第一段不受信代码、下载依赖、App、Browser 或 Backend 动作之前，Runtime 必须同时证明：frozen lockfile + `DependencyAcquisitionPolicy/Receipt` 的 dependency integrity；signed `RuntimeHardCeilings` 与 `ResourceLimitBinding/Receipt` 已应用；每次 DNS/redirect/connect 经 mediated gateway 产生 `NetworkFlowReceipt`；raw quarantine 已建立且 redaction 可强制；当前 Audit/Ledger checkpoint 已通过 verification；App、Browser、Backend、worker 与 VM 都处于可整体终止的 process domain。任一项缺失都拒绝进入 untrusted flow；任一 crash/restart 都只进入 admission-closed、read-only discovery，绝不在 M2 自动 resume。

每个 Local Design Sandbox 与 Local QA Sandbox 至少保证：

- 仅接受与已原子激活 LocalLeaseBinding 精确匹配的 Grant，并只导入本 Run 的固定 effective SHA、批准目录和必要只读输入。
- 每个 phase/generation 使用新 guest identity、临时可写 disk、guest agent session 和资源记录；Cleanup 后销毁，恢复或 Amendment 不直接复用。
- 默认不能读取用户主目录、其他仓库、浏览器个人 profile、keychain、未批准文件或宿主 loopback 服务。
- 网络目的地、文件导入/导出、Secret、CPU、内存、磁盘、vCPU、进程数和总时长同时受 Plan/Grant、VM 配置与 EffectGate 约束。
- Secret 只以 CredentialLease + ProcessLaunchBinding 交给精确 App/Middleware/backend process 或 authenticated proxy，不进入 control process、日志、任务正文、Artifact、普通 Event 或持久磁盘。
- Warden/Ledger 精确登记 VM、guest session、ExecutableIdentity、ProcessLaunchBinding、端口/proxy、Credential/materialization、Chrome/profile/control、quarantine 和 helper；Cleanup 前先 reconcile/seal，不能按模糊名称处理其他 Run 或用户进程。
- 外部 Fork PR 默认没有长期凭据、生产环境访问、宿主写权限或用户真实浏览器 profile。

### 10.4 Host Chrome Browser Provider

Browser Backend 是 `testing-runner` 选择的测试动作实现；browser process 是 Host Chrome Browser Provider 在宿主创建的专用 Chrome 进程；browser profile 是该 process 使用的每 generation 临时数据目录；control channel 是 EffectGate 授予 Host Chrome Browser Provider 的受限动作通道。四者必须分别建模和审计。

当测试需要宿主 Chrome 时，只能由 Provider 在 launch 前验证 bundle/Team ID/designated requirement/version/digest，创建专用 process和临时 profile，并强制使用 session-scoped deny-by-default egress proxy。Provider 对 redirect、subresource、DNS、WebSocket、service worker、download 和 popup 在连接时执行 envelope enforcement，并禁用 direct QUIC/WebRTC/PAC 等绕过。下载、截图、Trace 和 DOM/HTTP observation 先进入 quarantine/redaction，再进入 Artifact/Result。Worker 不得枚举/附加其他 Chrome、访问 CDP、proxy credential、个人 profile/keychain或直接宿主网络。

Cleanup 时，Supervisor 先撤销 Host Chrome Browser Provider control capability，再由 Process Warden 终止 Chrome process，关闭相关 helper，清理临时 profile，并在 Ledger 中结算 Receipt。POC 中由本地 Node.js Runtime 直接启动 Chrome 只证明 browser process 可达和动作可回传，没有证明 Host Chrome Browser Provider、EffectGate、profile 隔离、Process Warden 或 control channel 限制，不能作为生产边界。

### 10.5 Ledger、Process Warden 与 Update Module

SQLite Run Ledger 是本地恢复和所有权判定的唯一权威记录，不是普通日志缓存。Supervisor 必须以单写事务维护 reservation 到 activation 的状态转换、最高 accepted fence、active lease、effect 去重、资源句柄、revocation watermark、Checkpoint、Receipt、cleanup/repair settlement 和 update epoch。每个安全相关状态变化还必须追加 exact `AuditEvent`，按 `audit_sequence` 从 1 开始形成无间隙 hash chain；定期和关键事务边界生成 `AuditCheckpoint` 与覆盖 SQLite/WAL、audit、event outbox、effect、inventory、nonce/IPC/revocation sequence watermarks 的 `LedgerIntegrityCheckpoint`，再由独立 verifier 产生 `LedgerIntegrityVerificationReceipt`。任何无法原子持久化的副作用都必须采用 intent/receipt 对账协议，确保崩溃后能判定“未执行、已执行待确认或需补偿”，而不是由 Worker 猜测；integrity checkpoint 缺失、rollback、gap 或 root mismatch 必须保持 ordinary admission closed，禁止通过清空、跳过坏记录或只跑 SQLite pragma 恢复 healthy。

Process Warden 把 Ledger resource record 映射到可验证的 OS/Virtualization.framework 句柄。正常退出、取消、lease 到期、Supervisor 崩溃恢复、Amendment 和 Update 排空都使用同一套 quiesce、terminate、reap、verify-absent 流程。无法证明 ownership 的对象不得自动删除；已证明属于本 generation 的对象不能因 Worker 缺席而跳过清理。

Update Module 与 stable launcher 属于同一个 Runtime 部署目标。Update Module 停止新 reservation、stage 完整 compatibility set、验证 schema reader/writer window并请求 activation，但不能写 release selection。Stable launcher 独立记录 activation attempt，启动 candidate并等待与 attempt 绑定的 signed health evidence；成功才 commit current，candidate crash-before-health 或 timeout 时恢复未撤销且 schema-compatible 的 previous release。Rollback 后 Runtime 进入 recovering/recovery-only，沿统一 recovery protocol Snapshot/reconcile/Cleanup并等待 Hosted Decision，禁止自动恢复执行。

## 11. Structured Plan、Action Envelope 与 Plan Amendment

Structured Plan 是执行授权的可审查对象。它表达要运行哪些 Step、使用哪类 Backend、需要哪些依赖和能力、采用哪些结构化断言，以及每个动作允许触及的文件、网络、Secret 与资源范围。完整对象定义由 [SPEC.zh-CN.md](SPEC.zh-CN.md) 维护。

Action Envelope 是 Plan 中可执行权限的边界。Backend 可以在 envelope 内做受限探索，例如在批准目录中读取额外日志，或在已批准域名与预算内重试请求；它不能自行把“诊断需要”解释为新增权限。

以下变化必须触发 Plan Amendment：

- 新增、删除或改变会产生不同副作用的测试 Step。
- 扩大文件读写或挂载范围。
- 增加网络目的地或改变网络访问级别。
- 引入新的 Secret、凭据用途或更长暴露周期。
- 提升 Shell、浏览器、Agent 或宿主权限。
- 明显提高 CPU、内存、磁盘、进程数或总时长预算。
- 改变断言语义或会影响用户批准含义的 Backend。Source revision 变化禁止走 Amendment，必须创建新 Run。

Amendment 采用 hard-revoke 和新 generation 模型，完整顺序如下：

1. EffectGate 发现 EffectRequest 超出当前 Action Envelope，在任何宿主副作用前拒绝并回传 `amendment_required` 事件；Backend 不能先执行再补审批。
2. Supervisor 在幂等边界关闭该 generation 的非清理 EffectGate，要求 TypeScript Worker 与 Execution guest quiesce，撤销当前 CredentialLease，并在单写 Ledger 中保存带 generation/fence 的 Checkpoint、Artifact 摘要和精确 effect/resource ownership。
3. Hosted Authorization Authority 撤销旧 Execution Grant，Hosted 明确推进 generation/fence，使旧 command、旧 Worker/guest 和延迟 Receipt 不能继续产生有效副作用。为后续 generation 创建 reservation 之前或期间，旧 active generation 的 fencing 只能来自这次明确撤销/推进，不能来自 reservation 本身。
4. Supervisor 使用 CleanupCapability，以 `amendment_pause` 原因经 Process Warden 清理旧 Execution guest、Environment、Chrome process/profile、worker、helper、端口和 CredentialLease，并在 Ledger 中结算 Cleanup Receipt；Cleanup 不依赖旧 Execution Grant 仍有效。
5. Policy Gate 检查 amendment design scope，ExternalApprovalProvider 产生新的 DesignApprovalEvidence。Hosted 为新的 Design generation 请求 reservation 并取得 LocalLeaseBinding；Hosted Authorization Authority 验证 Evidence、Policy Decision 与 binding 后签发新的 Design Grant。
6. Supervisor 原子激活 Design reservation，创建全新的 Design guest 和 TypeScript design Worker，在原 RunSpec 和固定 effective SHA 上读取已批准的 Checkpoint/Evidence，生成 Plan vN 与相对 Plan vN-1 的结构化 Diff，然后执行 Design Cleanup。
7. Policy Gate 审查新 Plan 与 Diff；ExternalApprovalProvider 生成新的 ExecutionApprovalEvidence。Hosted 为新的 Execution generation 请求独立 reservation；Hosted Authorization Authority 验证 Evidence、Policy Decision 与 LocalLeaseBinding 后签发绑定新 `plan_digest`、generation、fence 和 reservation 的 Execution Grant。
8. Supervisor 原子激活 Execution reservation，创建全新的 Execution guest、Worker、Environment 和按需 Host Chrome Browser Provider 资源；只复用 Checkpoint 中已通过 Receipt 确认、在新 Plan 下仍有效且无需重跑的逻辑结果，不复用旧 guest、可写磁盘、CredentialLease、Chrome profile 或进程。所有后续宿主动作经 EffectGate 执行。

禁止在旧 Grant 撤销后直接恢复旧 Local QA Sandbox，禁止复用旧 CredentialLease、guest 可写状态或 browser profile，也不能重复已完成的外部副作用。未激活 reservation 可取消或过期，但不得清理、停止或 fencing 仍 active 的旧 generation。Source revision 变化不属于 Amendment，必须创建新 Run，并使现有 Approval Evidence、LocalLeaseBinding、Grant 和 Plan 绑定全部失效。

不可变 Plan 会增加交互等待，但它把 Agent 的开放式能力限制在用户真正看过的范围内。对高权限本地执行而言，这个等待成本低于静默扩权的风险。

## 12. Evidence、质量裁决、Outcome 与路由

### 12.1 Runner 与 Quality Evaluation 的分工

`testing-runner` 对每个 Case 的结构化断言负责，并产出 Pass/Fail 或无法完成的执行结果。Backend 只提供观察、日志、页面状态、命令退出状态和动作结果，Codex 的自然语言结论不构成测试 Oracle。

`quality-evaluation` 不重新执行断言。它聚合 Case Result、计划覆盖、Evidence 充分性和失败上下文，区分产品缺陷、测试资产问题、覆盖缺口、环境失败、Flaky、Policy Block 和 Evidence 不足，并生成 Final Quality Outcome。`test-publication` 只能消费 QualityEvaluation，不能直接根据原始日志创建产品 Issue。

### 12.2 五类 Outcome

Workflow state 与以下五类 Outcome 独立记录：

| Outcome | 回答的问题 |
| --- | --- |
| `execution_outcome` | 测试执行本身是 passed、failed、cancelled、timed_out 还是 lost |
| `cleanup_outcome` | 资源是否全部释放，是否部分成功或仍有残留 |
| `evidence_outcome` | 证据是否 sufficient、partial 或 insufficient |
| `publication_outcome` | 外部发布是 published、partially_published、failed 还是 skipped |
| `final_quality_outcome` | 对本次固定 source revision 的最终质量结论是 pass、fail、blocked 还是 inconclusive |

例如，测试可以通过但 Cleanup 部分失败；测试可以失败但 Evidence 不足，从而最终质量为 inconclusive；GitHub Publication 失败只能改变 `publication_outcome` 和对应 action settlement，不能改写已经完成的测试断言、`execution_outcome` 或 `final_quality_outcome`。分离 Outcome 能让重试和 repair 针对正确阶段进行。

### 12.3 GitHub 与 PQL 路由

| 分类或结论 | GitHub 路由 | PQL 路由 |
| --- | --- | --- |
| 所有可发布 Run | 幂等更新 PR Check、Comment 或 Run Summary | 无需默认创建资产变更 |
| `product_defect` | 只有可复现、Evidence 充分且已排除环境/测试资产问题时，才在被测仓库创建或更新 Issue | 不创建 PQL 产品缺陷 Issue |
| `test_failure` | 在 Run Summary 中说明测试资产问题；不误报产品 Issue | 创建/更新测试资产问题或 Asset Change Proposal |
| `coverage_gap` | 可在 Check/Summary 中说明未覆盖范围 | 创建 Coverage Gap，经 Review 形成 Asset Change Proposal |
| `environment_failure` | 保留 Run 记录；重复或平台性故障可进入 FKST 平台 Issue | 仅在测试资产需要环境声明修正时反馈 |
| `flaky` | Check 标记不稳定，不直接创建产品缺陷 | 更新 Flaky 记录并触发稳定性改进提案 |
| `policy_blocked` | 记录 Blocked 原因和审计引用，不创建产品 Issue | 通常不创建 PQL Proposal，除非资产持续请求不允许的能力 |
| `insufficient_evidence` | 标记结论不充分，避免发布确定性产品缺陷 | 创建 Evidence/Coverage Gap，推动补充日志、Trace、截图或断言 |

自动生成的新用例或资产修改必须先保持 `design_only`，经过 PQL Review 后才能进入新 Project Pack，并只影响后续 Run。任何反馈都不能在当前 Run 中绕过 Plan Amendment 自动变成可执行 Step。

### 12.4 Settlement、Terminal 与 Repair

Cleanup 与 Publication 都由一组可枚举 action 构成，每个 action 必须进入 `succeeded`、`failed`、`skipped` 或 `repair_queued` 之一，才算 settled。`failed` 和 `repair_queued` 不是成功，但它们表示本次 Durable Run 已记录确定结果、重试上限、责任方和稳定 repair key，不再处于含糊的进行中状态。

Run 只有在执行与质量计算完成，所有必需 cleanup/publication action 均 settled，并且最后接受的 Receipt、Checkpoint 与 effect ledger 已对账后才能进入 terminal。Terminal 因而表示“本次编排已收敛”，不表示测试、清理、证据或发布全部成功。

Terminal 后允许 repair operation 按稳定 repair key 重试 Cleanup 或 Publication。本地纯 Cleanup repair 由同一 `apps/local-qa-runtime` 部署目标内的 Rust Supervisor、EffectGate、Process Warden 和 trusted Adapter 执行，不启动 TypeScript test worker；Publication repair 属于 hosted-control-plane，二者不是新增部署目标。Repair 必须使用新的 repair attempt/generation 和 fence；纯 Cleanup repair 使用既有收窄 CleanupCapability，不需要也不得伪造新 Execution Grant；若 repair 确需新的非清理副作用，则必须先取得 LocalLeaseBinding，并由 Hosted Authorization Authority 签发相应收窄 Grant 后原子激活。Repair 追加不可变 per-attempt Receipt，并更新 cleanup/publication 的派生投影视图；它不重开原 Run，不重新执行测试，不改变固定的 Final Quality Outcome，也不能让旧 command、旧 lease、旧 guest 或旧 Worker 恢复有效。

## 13. 可靠性、安全、兼容性与权衡

### 13.1 可靠性、幂等与 settlement

Durable Orchestration 必须把每个外部副作用作为可识别 effect。Hosted 重试不应重复创建本地 generation，Runtime 重试不应重复激活 reservation、创建 guest 或启动服务，Artifact 重试不应重复上传内容，Publication 重试不应重复创建 Check、Comment 或 Issue。每个 effect 同时绑定 `run_id`、phase、action key、generation 和 fence；幂等键相同但 generation 过旧的请求必须被拒绝，而不是被当作当前重试。

本地 effect 采用 Supervisor 单写 Ledger 中的 intent/receipt 协议。EffectGate 在执行前原子写入 intent 与 ownership，执行后写入 Receipt；Process Warden 为 VM、worker、Chrome、helper 和端口保存可重新发现的 OS 句柄。Prepare、Readiness、Step、Cleanup、Artifact、Publication 和 Repair 应通过稳定标识、输入/输出摘要、generation/fence 与 Checkpoint 对齐。Runtime 或 Hosted 崩溃后，系统先对账再决定返回既有结果、补写 Receipt、执行补偿、推进 generation、settle failure 或进入 repair，不能仅根据 Worker 消息或“最后一个状态名称”推断副作用是否发生。

所有跨 Hosted、NyxID Transport 与 Runtime 的 reservation request、LocalLeaseBinding、Grant、command、event、Receipt 和 Checkpoint 都必须显式绑定 Run/phase/device，并在适用时绑定 generation 与 fence。reservation 只允许创建未激活 binding，不能改变当前 accepted fence；只有 Hosted Authorization Authority 签发的匹配 Grant 经 Supervisor 原子激活后，才允许更高合法 fence 推进本地执行权。旧 generation 的消息可以作为审计事实留存，但不能覆盖新 Checkpoint、延长 lease、提交新 Evidence、确认 Cleanup 或改变 settlement。

### 13.2 安全与契约兼容

跨边界对象以及 Supervisor/Worker/guest/browser 内部协议需要版本、内容摘要、RunSpec 关联、generation、fence 和生产者版本，以便拒绝篡改、错误绑定、延迟重放和不兼容消息。签名 Grant 需要防重放、短有效期、目标设备、LocalLeaseBinding 和 audience 约束；reservation TTL、atomic activation、撤销状态、hosted/local lease、fencing 和 Runtime 本地 nonce 记录共同阻止重复执行。TypeScript Worker 或 guest compromise 不能取得 Grant 签发密钥、Ledger 写权限、Supervisor IPC identity、宿主 Secret 明文存储或任意 Browser control channel。

Raw observation 与 Artifact 默认先进入 bounded local quarantine，完成 redaction、sanitized validation 后才可进入普通 Ledger/Event/CaseResult 或离开设备；Artifact identity 和幂等键基于最终 post-redaction bytes。SPEC 锁定 v1 的 envelope encryption、短期 access capability、默认 retention profile、legal hold 和删除证明语义；具体 Artifact Store provider 可以替换，但不得削弱这些约束。脱敏失败只能产生 safe Receipt/blocked Evidence，不能持久化或上传原始内容。

### 13.3 主要权衡

| 决策 | 获得 | 代价 |
| --- | --- | --- |
| 用户本地执行 | 真实依赖、宿主 Chrome 和开发环境；无需把全部源码与 Secret 迁到云端 | 设备异构、离线、升级和资源争用更难处理 |
| 两个 app 独立部署 | 云端与本地可独立扩缩、升级和设置信任边界 | 需要严格契约兼容和跨版本测试 |
| 本地 app 单一部署目标 | Supervisor、Worker、VM、Browser、Secret、Ledger 和更新共享签名、版本与运维入口 | app 内 IPC、权限降级和组件兼容必须设计严谨，不能靠部署拆分掩盖边界 |
| Hosted Authorization Authority 与 NyxID 分离 | FKST 保有业务授权语义，NyxID 可替换，审计责任清晰 | 增加 Approval Evidence、LocalLeaseBinding、撤销和密钥管理协议 |
| Grant 前 LocalLeaseBinding | Grant 绑定实际设备、Runtime 能力和本地容量，并可原子激活、防止幽灵 Dispatch | 增加 reservation TTL、取消、并发和崩溃恢复状态；必须证明 reservation 不 fencing active generation |
| 用户级 LaunchAgent + Rust Supervisor | 符合用户会话和宿主 Chrome 边界，提供内存安全控制面、单写 Ledger 与稳定 launchd 生命周期 | 初期仅覆盖 macOS 登录用户场景，需要 Rust/TypeScript IPC、签名和升级基础设施 |
| 每 generation Virtualization.framework Linux guest | Design/Execution 与不同 generation 具有明确内核隔离和可销毁状态 | VM 启动、镜像分发、磁盘占用、Apple Silicon/x86 兼容和本地资源成本更高 |
| TypeScript Worker + mediated EffectGate | 复用 testing 生态，同时把宿主能力集中到可审计 Rust 执行点 | 所有必要宿主能力都要有类型化协议；未中介能力必须由 VM/OS 配置明确封闭 |
| Host Chrome Browser Provider | 保留真实系统 Chrome，同时隔离个人 profile/keychain 并限制 browser control | 浏览器位于 guest 外，需处理文件交换、网络关联、生命周期和证据归属 |
| Single-writer SQLite Ledger + Process Warden | 本地事实源、资源所有权和崩溃恢复可确定对账 | Supervisor 成为关键可靠性组件，Schema migration、WAL 恢复和句柄再发现必须经过故障注入 |
| PR synthetic merge commit | 默认验证合入结果并锁定可重放 revision | 需要处理冲突、对象保留和 merge 元数据 |
| Immutable Plan + Amendment | 用户批准与实际动作一致，Agent 不可静默扩权 | 复杂诊断可能需要再次等待批准和创建新 guest |
| Packages 留在 monorepo | 原子更新契约与模块，减少跨仓库漂移 | 必须用依赖规则防止 apps 与 packages 反向耦合 |
| 独立 Quality Evaluation | 产品缺陷、测试问题和环境问题不会混为一谈 | 发布前多一个需要版本化和测试的裁决阶段 |

仍需在实现 ADR 中选择、但不得改变上述边界的政策只包括：满足 SPEC Artifact 契约的具体 Store/KMS provider、离线设备等待上限，以及哪些低风险 Design 权限可以由组织策略预批准。Runtime 更新由 Update Module 负责这一架构责任已经锁定；具体发布节奏和强制窗口属于运维策略。Sandbox Provider、LaunchAgent/Supervisor 形态、Worker 语言、Ledger 写入模型、EffectGate、Host Chrome Browser Provider、Artifact 加密/访问/保留/删除语义和每 generation Linux guest 不再是待定项。

## 14. 实施阶段与架构验收标准

### 14.1 实施阶段

| 阶段 | 目标 | 主要产物 |
| --- | --- | --- |
| M0：契约、Source 与信任根 | 锁定所有跨边界 strict contract、签名投影和兼容语义 | RFC 8785/JCS corpus、RunDraft→SourceAcquisition→RunSpec、DeviceAttestation/Approval/Grant、RuntimeIdentity/Pairing、RevocationBatch、RuntimeAdmissionSnapshot/AdmissionRequirements、Dependency/HardCeilings/ResourceLimit/Network/Redaction/Audit exact contracts、八方法 Runtime Interface，以及 Cancellation/Timeout/Recovery/Update exact contracts |
| M1：macOS Runtime 控制面 | 建立可安装、可认证、可审计且只有一个本地事实源的 Runtime 控制面 | Hosted Authorization Authority、stable launcher + signed Rust release、Runtime identity key + pair/re-pair/revoke/reset、single-writer SQLite、epoch-bound LocalIPCBinding、strict pre-Grant reservation/atomic admission、独立 signed revocation control inbox、event/audit outbox、Audit/Ledger integrity checkpoints、RuntimeHealth/degraded operation matrix 与 activation journal |
| M2：首个完整 untrusted-flow Gate | 在任何不受信代码、依赖、App、Browser 或 Backend 动作前一次性建立不可绕过的执行安全基线 | 独立 Design/Execution VZ VM、GuestBootEvidence/boot-bound authenticated vsock、phase EffectGate、ProcessDomainDescriptor/Warden、Secret Broker、probe-gated BrowserProvider 与 EnvironmentFactory；同时强制 dependency integrity、Runtime hard limits、per-flow egress、raw quarantine + enforceable redaction、Audit/Ledger checkpoint 和 process/VM crash containment，crash/restart 后 admission closed、只读 discovery、绝不 auto-resume |
| M3：完整恢复、Resume 与 Amendment | 补齐 hosted/local lease、恢复、取消、清理和扩权后的完整可恢复生命周期 | execution/control_quiesce_reconcile/control_cleanup transition、first cursor/ack、canonical EffectState/outbox、Checkpoint/Snapshot、split Amendment/Recovery Resume、signed RecoveryDecision、完整 Amendment reapproval、新 Sandbox、Termination、seal/successor Cleanup、terminal repair 与 migration-aware rollback |
| M4：证据、质量与发布 settlement | 交付 advanced Artifact/Evidence、独立 Quality 和外部发布 settlement | 类型化 Evidence fulfillment、完整 Artifact redaction/access/retention/delete lifecycle、版本化 QualityEvaluation、GitHub/PQL Publication Action、PublicationReceipt、settlement/repair 与全生命周期 failure injection |
| M5：PQL Loop | 把测试缺口转化为经过 Review 的下一版资产，且不反向改变当前 Run | CoverageGap、immutable AssetChangeProposal、PQLReviewDecision、ProjectPackPromotionReceipt、并发 pack conflict 与下一轮回归验证 |

M0-M5 是唯一系统级里程碑编号。`LOCAL-QA-RUNTIME-DESIGN.zh-CN.md` 使用 Runtime R0-R3 表达本地交付增量，二者不是一一对应：R0 交付系统 M0 以及 M1 的 Ledger foundation；R1 完成 M1 的 launcher/control plane并交付 M2 的 Guest/Effect/Secret/Browser 安全执行；R2 完成 M3 的取消、超时、恢复和 Amendment；R3 提供 M4 所需的制品/观测输入，并补齐横跨 M1-M3 的生产更新、磁盘和长期运行 Gate。Runtime 编号不得被解释为独立的系统路线图。

M1 应复用 Chrome POC 的传输结论，但不能复用其无认证服务、内置 fixture、Node.js 进程兼任 Supervisor、直接宿主执行、个人 profile 或开放 control channel 作为生产安全模型。M1 可以使用最小 Artifact Store，只建立 signed release selection、staging/activation journal 和 crash-before-health rollback 基线；advanced Artifact fulfillment/access/retention/delete 属于 M4，update 的 migration-aware rollback 属于 M3，更完整的 rollout/migration/disk/outbox failure matrix 在这些前置 Gate 后成熟，不能被塞进 M2 来替代首个 untrusted-flow security proof。

M0 到 M3 是进入真实项目试运行的前置条件；Browser Plan 还必须通过全 Chrome process tree direct-socket denial Gate，未通过时 Runtime 只能广告非 Browser capability。M2 必须同时通过 dependency substitution、hard-ceiling bypass、逐流 egress bypass、raw quarantine/redaction、Audit/Ledger integrity 和 process/VM crash containment 测试；完整 Resume 与 Amendment 只能在 M3 通过 signed RecoveryDecision、重新授权和新 Sandbox 实现。在 LocalLeaseBinding 并发/崩溃测试、EffectGate 绕过测试、VM/Chrome/worker 强制清理和 migration-aware rollback 通过前，不得把 Runtime 声明为生产可用。M4 完成前不得把所有失败自动发布为产品 Issue，也不得把 Publication 成功当作 Quality 成功；M5 完成前只允许人工维护 PQL 反馈。

### 14.2 架构验收标准

实现进入生产前，至少必须满足以下条件：

- 系统级 Mermaid 是架构语义基准，定制 SVG 是同步 presentation view；[Local QA Runtime 详细设计](LOCAL-QA-RUNTIME-DESIGN.zh-CN.md) 及 [内部 Mermaid](fkst-local-qa-runtime-internals.mmd)/[SVG](fkst-local-qa-runtime-internals.svg) 细化本文锁定的 host/guest/worker/browser/secret/ledger 边界，自动或人工检查能发现标签、边界、授权方向和部署目标漂移。
- PR 默认生成固定 synthetic merge commit；非 PR Run 使用 exact commit SHA；系统按 RunDraft→SourceAcquisition→RunSpec 冻结身份，Plan、Approval Evidence、Grant、Evidence 和发布结论绑定同一 effective SHA。
- revision 变化创建新 Run，并使旧 DesignApprovalEvidence、ExecutionApprovalEvidence、Design Grant、Execution Grant、Plan 绑定和待提交 Evidence 失效。
- Design 预批准也产生 Evidence；每次 Design/Execution Grant 前，Runtime 按 strict phase authorization preimage 和 AdmissionRequirements 建立 inert reservation/binding。Authority 验证 Evidence、Policy 与 binding 后签 Grant；Design binding/Grant 禁止出现 Execution Plan/Policy/envelope 字段。
- reservation 创建、续短、取消或过期不推进 fence或影响 active generation。首次 command admission 先完成幂等 lookup，再原子创建 stable environment、empty inventory root、完整 CleanupCapability、LocalExecutionLease/FenceTransition、CommandAdmissionReceipt、initial effects和 sequence=1 outbox；失败不留下部分状态。
- 用户批准的是 Design scope 或 Structured Plan/Execution scope；LocalLeaseBinding 只证明本地预留；ExternalApprovalProvider 只提供批准/设备证明，NyxID Transport 只传输，二者都不签发 Grant、不制造 binding、不修改 scope、不扩大 Secret 权限。
- Policy Gate 与 Local PEP 分别完成云端策略判断和本地逐动作强制执行；v1 Local PEP 由 Rust Supervisor 管理的 mediated EffectGate 实现。Local Runtime Verifier 只验证授权与 binding 有效性，不能替代 EffectGate。
- `apps/hosted-control-plane` 与 `apps/local-qa-runtime` 独立构建和发布；本地 LaunchAgent、Rust Supervisor、TypeScript Worker、VM/helper、EffectGate、Process Warden、Secret Broker、Host Chrome Browser Provider、Ledger 与 Update Module 全部属于 `apps/local-qa-runtime` 的一个签名部署目标；`packages/*` 不是部署目标且不依赖 apps 实现。
- Runtime v1 以用户级 LaunchAgent 固定启动 stable launcher，再选择签名 Rust Supervisor；无 LaunchDaemon。Local IPC 和 guest vsock 双向认证并绑定 executable/boot/generation；health 暴露 recovering/admission/capacity/disk；launcher 能在 candidate API 启动前失败时完成回滚。
- Runtime bootstrap 创建 device-bound non-exportable key 和 `RuntimeIdentityStatement`；initial pair、identity rotation continuity + forced re-pair、ordinary re-pair、pairing revoke与 emergency reset 都按 SPEC epoch 规则实现。旧 identity/pairing epoch 的 DeviceAttestation、Grant、reservation、IPC session 与 revocation ack全部 fail closed，NyxID Node identity 不得替代 Runtime identity。
- `RuntimeService` 精确且仅有 `probeHealth`、`reserveLocalLeaseBinding`、`cancelReservation`、`submitCommand`、`getRun`、`streamEvents`、`ackEvents`、`getArtifact` 八个方法；`RuntimeTransportControlInbox.deliverRevocations` 独立接收 signed batch并返回 durable idempotent ack，不是第九个方法，也不能传 command/config。
- Revocation freshness、batch chain 或 watermark 不满足要求时，Runtime 关闭相应 admission/effect/Artifact read path并请求 snapshot/re-delivery；同 batch 重放返回原 Receipt，ack 丢失不重复 apply，ack 不代表 Cancel/Cleanup settled。
- 同一 UID 防护验收只声明 tamper detection + fail-closed，不声明 DoS prevention；测试必须区分 binary/Ledger/session/checkpoint 篡改后的关闭行为与同 UID kill、资源耗尽、网络阻断等可用性攻击。
- 每个 Design generation 与 Execution generation 都创建独立 Virtualization.framework Linux guest、临时可写 disk 和 guest agent session，并分别授权、记账、清理和销毁；Design Grant 不能启动正式测试，Execution Grant 不能复用 Design guest，新 generation 不能复用旧 generation 的可写 guest 状态。
- Workspace 仅表示固定 source 文件视图，Sandbox 表示 generation 独占 Linux guest 隔离边界，Environment 表示 Execution guest 内运行资源；宿主 Chrome 是受控外部资源；实现、Schema 和日志不混用这些概念。
- TypeScript Worker 只在 phase guest 内运行，为非特权、随 VM 回收；只能经 boot-bound authenticated vsock 提交 typed PlanAction/EffectRequest，不能写 Ledger、调用 VZ、读取 Secret material、启动宿主进程、连接 Chrome/CDP或取得 proxy credential。
- 所有副作用使用 phase-specific EffectContext、typed PlanAction/BrowserAction/EffectRequest 经过 EffectGate；root-qualified path、VM/OS 配置和 control-cleanup purpose 封闭未中介路径，默认不能读取 home、其他 repo、宿主 loopback或个人浏览器状态。
- SQLite Run Ledger 只有 Rust Supervisor 一个写入者；reservation/activation、最高 fence、effect intent/receipt、resource ownership、revocation watermark、Checkpoint 和 settlement 使用事务更新，Worker/guest/helper 无法维护竞争性事实源。`AuditEvent` 是无间隙 append-only signed hash chain；`AuditCheckpoint` 与 `LedgerIntegrityCheckpoint/VerificationReceipt` 覆盖 SQLite/WAL、audit、outbox、effect、inventory 和 nonce/sequence watermarks，integrity failure 关闭 admission且不能通过清空历史恢复。
- Process Warden 使用 ExecutableIdentity、ProcessLaunchBinding、PID/start token、FD/VZ handle 和 owner tag；在崩溃、取消、Amendment、更新/重启后先 reconcile再 seal/cleanup，既不误杀用户进程，也不遗漏 late-discovered resource。
- Secret materialization 同时绑定 CredentialLease、ProcessLaunchBinding、ExecutableIdentity、actual ProcessIdentity、Step/destination/fence；Supervisor 只处理 opaque ref 与签名 receipt。proxy mode、guest injector 和 env/file mode 分别声明明文 custodian、descendant inheritance、core dump/swap/logging 与擦除规则；unknown release/revoke 形成 blocking residual。
- Browser Provider 启动前验证 Chrome identity，每 Run/generation 使用临时 profile、typed channel 和 forced proxy；Browser enforcement probe 必须证明完整 Chrome process tree 的 IPv4/IPv6 TCP/UDP 直连被拒绝并产生可关联 telemetry，否则不广告 Browser capability。运行中 enforcement 丢失必须 fail closed；Browser output 先 quarantine/redact再进入 Result/Artifact。
- Update Module 只 stage/drain/preflight并请求 activation；stable launcher 验证 manifest/security epoch/schema window，启动 candidate、等待 signed health evidence并 commit/rollback。rollback 不自动 resume，主 Ledger 不可写时 activation journal 仍保留审计。
- RuntimeHealth 的 recovering/admission/capacity/disk/outbox 状态具有明确 operation matrix；soft/hard watermark 拒绝相应工作，并始终预留 Ledger/WAL/fence/inventory/Cleanup Receipt emergency headroom，`SQLITE_FULL` 不允许继续执行或删除权威事实。
- `testing-runner` 根据结构化断言决定 Case Pass/Fail；Codex 只能在批准的 Action Envelope 内执行，超出范围的 EffectRequest 由 EffectGate 在宿主副作用前阻断。
- M2 首个完整 untrusted-flow Gate 在 Design/Execution 第一段不受信代码、依赖、App、Browser 或 Backend 动作前，同时验证 dependency integrity、signed RuntimeHardCeilings/ResourceLimit、per-flow egress、raw quarantine + enforceable redaction、Audit/Ledger checkpoint 和 process/VM crash containment；任一 crash/restart 只进入 admission-closed read-only discovery，禁止 auto-resume。
- 完整 Resume 与 Amendment 在下一里程碑 M3 交付：Amendment 完整执行 quiesce/checkpoint、旧 Grant revocation、`control_quiesce_reconcile` takeover、reconcile/seal，再以独立 `control_cleanup` takeover执行 Cleanup；随后 Design 和 Execution 各自严格按 reserve→binding→Grant→command admission，冻结完整 PlanAmendment 后才提交 amendment ResumeCommand并创建新 guest。Recovery 使用独立 signed RecoveryDecision 和 RecoveryResumeCommand，禁止与 Amendment Resume 混用。
- Readiness 按 Plan 条件执行，不把 Host Chrome Browser Provider 准备强加给 API、CLI 或单元测试。
- Cleanup 前必须 quiesce、reconcile 并生成 sealed inventory/InventorySealReceipt；CleanupCommand 绑定 lineage/ref/version/digest。Capability 过期后只允许同 lineage、等权/更窄 successor；成功、失败、取消、重启、rollback 和 Amendment 都能独立收敛。
- signed RecoveryDecision、execution/control-cleanup FenceTransition、predecessor fencing、LocalLeaseBinding、CommandAdmissionReceipt 和 first-cursor rule 阻止旧 Runtime/lease/guest、延迟消息和错误-purpose takeover提交副作用；same-key retry不重复消费 nonce。
- Return 是贯穿 Design、Execution、Cleanup、Quality 和 Publication 的事件/Receipt 回传流，不是末尾一次性结果返回。
- `execution_outcome`、`cleanup_outcome`、`evidence_outcome`、`publication_outcome` 和 `final_quality_outcome` 独立记录，不与 workflow state 或 action settlement 混用。
- `quality-evaluation` 位于 Artifacts 与 Publication 之间，能区分产品、测试资产、覆盖、环境、Flaky、Policy 和 Evidence 问题；Publication 失败不改变 Final Quality Outcome。
- Run 只有在所有必需 cleanup/publication action 已 settled 后才 terminal；terminal 可包含失败或 repair backlog，不要求全部成功。
- Terminal 后纯 Cleanup repair 使用 signed RepairOperation、control-cleanup FenceTransition、capability successor、stable repair key、新 attempt/fence 和不可变 RuntimeRepairReceipt；不启动 test worker、不创建旧 inventory 外 VM/资源、不重开 Run或改变 Quality。需要新工具必须单独授权为 non-cleanup repair。
- GitHub Check/Comment/Issue 与 PQL Gap/Proposal 使用稳定去重和 repair key；重放同一 Run 不重复启动服务、上传 Artifact 或创建外部对象。
- 产品缺陷只在可复现且 Evidence 充分时进入被测仓库；测试失败、覆盖缺口、Flaky 和 Evidence 不足按规则进入 PQL Review Loop。
- POC 只证明 Cloud → Node → 已运行 Runtime → Chrome → 结果回传；stable launcher、strict reservation/admission root、phase EffectGate、authenticated local IPC、boot-bound authenticated vsock、purpose-specific takeover、seal/capability successor、process-bound Secret、probe-gated verified/proxied Chrome、quarantine/redaction、disk pressure、update rollback 和 terminal settlement 必须通过 M0-M5 Gate 与故障注入后才能声明生产可用。

满足这些标准后，系统才是一条批准可证明、授权可验证、本地副作用可强制约束、状态可恢复、结果可裁决、外部发布可去重且可修复、测试资产可学习的长期 QA Loop，而不是一次远程触发本地脚本的演示链路。
