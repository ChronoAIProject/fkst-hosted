# FKST Host → NyxID → 用户本地自动化 QA 实现规范

> **文档状态：** 目标实现规范，尚未表示完整系统已经实现。  
> **适用版本：** v1，macOS-first Local QA Runtime。  
> **最后校准日期：** 2026-07-27。
> **配套设计：** [DESIGN.zh-CN.md](./DESIGN.zh-CN.md) / [LOCAL-QA-RUNTIME-DESIGN.zh-CN.md](./LOCAL-QA-RUNTIME-DESIGN.zh-CN.md)。
> **架构图：** [SVG](./fkst-host-nyxid-local-qa-flow.svg) / [Mermaid](./fkst-host-nyxid-local-qa-flow.mmd)。  
> **Runtime 内部图：** [SVG](./fkst-local-qa-runtime-internals.svg) / [Mermaid](./fkst-local-qa-runtime-internals.mmd) / [Excalidraw](./fkst-local-qa-runtime-internals.excalidraw) / [PNG](./fkst-local-qa-runtime-internals.png)。
> **评审依据：** [FKST-NyxID-Local-QA-Architecture-Review.md](./FKST-NyxID-Local-QA-Architecture-Review.md)。  
> **已验证 POC：** [NyxID-Local-Chrome-Minimal-Loop-Validation.md](./NyxID-Local-Chrome-Minimal-Loop-Validation.md)。

---

## 0. 规范语言、证据边界与阅读方式

本文中的规范词含义如下：

| 中文 | RFC 风格 | 含义 |
|---|---|---|
| **必须** | **MUST** | 实现不满足即视为不符合本规范。 |
| **禁止** | **MUST NOT** | 实现出现该行为即视为不符合本规范。 |
| **应该** | **SHOULD** | 除非有记录充分的例外理由，否则必须满足。 |
| **不应该** | **SHOULD NOT** | 除非有记录充分的例外理由，否则不得采用。 |
| **可以** | **MAY** | 可选能力，不影响基础合规性。 |

本文只把以下链路视为已被 POC 证明：

```text
NyxID Cloud
→ NyxID Node
→ 已经运行的本地 Runtime
→ 系统 Google Chrome
→ 浏览器交互与 DOM 断言
→ 截图和结构化结果
→ NyxID Node
→ 调用方
```

POC **没有**证明双阶段授权、隔离 Workspace、真实 App/Middleware 生命周期、补偿式 Cleanup、Runtime 自动安装、断线恢复、质量裁决、Artifact 上传、GitHub 发布或 PQL 学习闭环。上述能力均属于本规范要求的待实现和待验收范围。

本文不继承历史规范中的云端 OSB QA Sandbox、固定 2 CPU/4Gi、单个 unrestricted Codex step、Issue Comment 状态总线、浏览器 E2E 不可行、所有失败直接创建产品 Issue、独立 testing repository 等假设。

---

## 1. 范围、目标与锁定决策

### 1.1 目标

系统必须允许 `fkst-hosted` 创建可恢复的 QA Run，经用户批准后，通过可替换的设备传输通道在用户电脑上的 Local QA Sandbox 中执行自动化测试，并把结构化结果、Evidence、Cleanup Receipt、质量裁决和发布结果回传到云端。

### 1.2 锁定决策

1. **代码组织。** 实现必须位于 `fkst-hosted` monorepo。`apps/hosted-control-plane` 与 `apps/local-qa-runtime` 必须独立构建、签名、部署和升级；testing modules 必须位于 `packages/`，且 packages 禁止依赖任何 `apps/` 实现。
2. **执行位置。** Local QA Sandbox 必须位于用户电脑。PR 代码、依赖、Shell、浏览器、被测服务和 Agent Action 禁止在 NyxID 或 hosted control plane 内执行。
3. **源码版本。** PR Run 默认必须生成并记录固定的 synthetic merge commit；非 PR Run 必须使用 exact commit SHA。
4. **授权签发。** Execution Grant 必须由 `fkst-hosted` 内独立的 Authorization Authority Module 签发。NyxID 提供用户批准证明、设备证明、审计和传输，但禁止作为 Grant 签发权威。
5. **Runtime v1。** Local QA Runtime v1 必须是 macOS-first 的签名 Rust 主进程，作为当前登录用户的 user LaunchAgent 由 `launchd` 管理；禁止在 v1 改为 root LaunchDaemon。测试设计、Runner 和 Backend worker 必须使用 TypeScript，并由每阶段 Linux VM 的 guest agent 启停和监管，经受认证、版本化 vsock/guest protocol 与 Rust Supervisor 通信；TypeScript worker 禁止作为普通宿主进程运行、直接持有宿主副作用能力或写本地权威数据库。
6. **NyxID 边界。** NyxID 禁止执行测试、启动浏览器、Checkout 代码、安装依赖、运行 Shell、判断 Pass/Fail 或生成最终 Quality Outcome。
7. **测试裁决。** `testing-runner` 必须根据结构化 assertion 决定每个 Case 的 Pass/Fail。Backend 或 Codex 的自然语言自报结论禁止成为测试判定依据。
8. **计划变更。** 已批准 Plan 必须不可变。任何超出 `ActionEnvelope` 的动作必须通过 `PlanAmendment` 生成新版本并重新审批。
9. **Cleanup。** Cleanup 必须是补偿阶段，从成功、失败、取消、超时、失联和 Runtime 恢复路径触发，禁止依赖正常执行链成功。
10. **发布输入。** Publication 必须只消费 `QualityEvaluation` 和经授权的 Artifact Pointer，禁止直接根据 Backend 输出或原始日志创建产品 Issue。
11. **Sandbox Provider v1。** Design 与 Execution Sandbox 必须使用 macOS `Virtualization.framework` 托管的 Linux VM；VM image、guest agent、mount、网络与资源配置必须 digest-bound。v1 禁止把未受隔离的宿主进程、Container-only provider 或用户 Shell 当作等价 Sandbox。
12. **浏览器 v1。** BrowserProvider 必须启动宿主系统 Google Chrome，并使用每 Run 临时 Profile、宿主 Process Warden 和 provider-mediated automation channel；Chrome 禁止运行在 Linux VM 内，禁止向 worker、Plan、Event 或 hosted 暴露 CDP/debugging endpoint 或 token。
13. **本地执行权威。** Rust Runtime 内的 mediated Local PEP `EffectGate` 必须同时完成判定与副作用执行；禁止使用“先 authorize、后由 caller 自行执行”的 split API。所有宿主和 VM 副作用必须经 `EffectGate.perform`。
14. **本地持久化。** Runtime 权威状态必须持久化到单个 SQLite ledger。Rust Runtime 必须是唯一 writer；TypeScript worker、NyxID Node、guest agent 和 BrowserProvider 禁止直接写该数据库。实现必须使用事务、foreign key、busy timeout、WAL 或等价崩溃恢复配置和启动一致性检查。

### 1.3 非目标

- 本规范不要求 v1 支持 Linux 或 Windows Runtime。
- 本规范不要求 v1 支持 `Virtualization.framework` 以外的 Sandbox Provider；未来 provider 必须通过新 major contract 与独立安全验收引入，禁止在 v1 配置中静默替换。
- 本规范不锁定 Artifact Store/KMS 供应商；v1 默认和最大保留期由 §11.1 锁定，部署策略只能选择更短 TTL、在 maximum 内发布新 policy version，或通过显式 legal hold 延长。
- 本规范不要求 NyxID 成为唯一传输实现；本机 CLI、企业 Device Agent 或其他自托管通道可以实现同一 transport-neutral Runtime 协议。
- 本规范不允许 PQL 直接调度 Runtime、签发 Grant 或发布产品缺陷。

---

## 2. 系统不变量与权威边界

| 事项 | 唯一权威 | 约束 |
|---|---|---|
| Run 持久状态与状态迁移 | hosted workflow | Runtime 事件只能请求迁移，不能自行改写云端终态。 |
| RunDraft | hosted workflow | Source 尚未成功获取时只能持久化 Draft，禁止提前冻结 RunSpec。 |
| SourceAcquisition / SourceRevision | hosted source resolver | Resolver 必须冻结可重放对象；Runtime 必须验证 checkout 后的 `effective_sha`、对象 digest 和 resolver 绑定。 |
| 用户批准与设备证明 | NyxID 或其他 Approval Provider | 证明必须被 hosted Authorization Authority 验签和消费。 |
| Design/Execution Grant | hosted Authorization Authority | 私钥禁止进入 NyxID Node 或 Local Runtime。 |
| Structured Plan | testing-design | 一旦进入审批即不可原地修改。 |
| Step Pass/Fail | testing-runner | Backend 只返回观察结果和 Artifact。 |
| Sandbox 生命周期 | local Runtime + EnvironmentFactory | hosted 只能发命令和消费 Receipt。 |
| CleanupCapability | Local Runtime cleanup authority | Capability 必须在本地创建、最小授权并独立于 Execution Grant 过期；hosted 只能持有 digest-bound reference。 |
| CredentialLease | 独立 Local Secret Broker helper | Secret 值和 lease material 禁止进入 hosted、NyxID、Supervisor、Plan、Grant、普通事件或 Ledger payload。 |
| Final Quality Outcome | quality-evaluation | Publication 禁止自行推导质量结论。 |
| GitHub/PQL 副作用 | test-publication adapters | 所有副作用必须带稳定 dedup key。 |
| PQL Review Decision | PQL review authority | Review 必须是不可变 Decision，不得原地修改 Proposal 状态。 |
| Project Pack Promotion | PQL promotion authority | 只有 PromotionReceipt 可以使已批准资产进入新的可执行 Project Pack。 |

以下边界必须成立：

```text
Approval Provider 证明“谁批准、批准哪台设备、何时批准”
Authorization Authority 决定“批准证明是否足以签发哪种 Grant”
Local Runtime 验证“Grant 是否允许本机执行这个 Plan”
Local PEP 决定“当前动作是否同时满足 Plan、Policy、Grant、Fence 与本地能力约束”
testing-runner 判断“结构化断言是否通过”
quality-evaluation 判断“整个 Run 对产品质量意味着什么”
PQL review/promotion authorities 决定“提案是否获批并进入哪个不可变 Project Pack”
```

---

## 3. 公共契约规则

### 3.1 公共元数据

所有跨 module、跨进程或跨设备持久化对象必须包含以下字段：

```ts
type ISO8601 = `${number}-${number}-${number}T${number}:${number}:${number}${"Z" | `.${number}Z`}`; // UTC RFC3339；禁止 offset；fraction 无尾随 0
type Sha256 = `sha256:${Lowercase<string>}`; // `sha256:` + 64 个 lowercase hex
type Base64UrlNoPad = string;              // RFC 4648 base64url，无 `=` padding
type UUID = string;

type ContractMeta = {
  schema_version: string;       // 例如 "qa.runspec/v1"
  content_digest: Sha256;       // JCS payload UTF-8 bytes 的 SHA-256
  run_id: UUID;
  created_at: ISO8601;
  producer_version: string;     // semver 或不可变 build id
  correlation_id?: string;
};

type RuntimeScopedMeta = {
  schema_version: string;       // 例如 "qa.runtime-health/v1"
  content_digest: Sha256;
  runtime_instance_id: string;
  created_at: ISO8601;
  producer_version: string;
  correlation_id?: string;
};
```

`ContractMeta` 只用于归属单个 Run 的对象。健康、升级和 Runtime 自检等不归属单个 Run 的对象必须使用 `RuntimeScopedMeta`，禁止伪造 sentinel `run_id`。

### 3.2 RFC 8785 / JCS Canonical Serialization

1. 所有 `content_digest`、`DigestBoundRef.content_digest`、签名 payload、幂等 request digest 和本规范所称 canonical digest，必须使用 RFC 8785 JSON Canonicalization Scheme（JCS）生成的 UTF-8 bytes；禁止使用“对象 key 字典序 + `JSON.stringify`”等近似实现。
2. 输入必须满足 I-JSON 约束。解析器必须在对象物化前拒绝重复 member name，必须拒绝非法 UTF-8、lone surrogate、`NaN`、正负 `Infinity` 和非 JSON 数值；合法 JSON number 必须按 IEEE 754 binary64 解析并按 JCS 重新序列化，因此输入十进制 token 不保证逐字符保留。具有精确整数语义且超出 `[-9007199254740991, 9007199254740991]` 的字段必须由 schema 定义为十进制字符串；实现检测到整数 token 已发生精度丢失时必须拒绝，禁止先舍入再作为权威整数 canonicalize。
3. JCS 数字序列化必须遵循 RFC 8785 引用的 ECMAScript `NumberToString` 结果，包括 `-0` canonicalize 为 `0`、有限数的最短 round-trip 十进制表示和规定的 exponent 格式。实现禁止依赖语言默认 decimal formatter。
4. 对象 member name 必须按 RFC 8785 规定的 UTF-16 code unit 顺序递归排序；数组必须保持原顺序；字符串 escaping 必须遵循 JCS。字符串值和 member name 禁止做 NFC、NFD、NFKC、NFKD、case folding 或任何其他 Unicode normalization；不同 code point 序列即为不同输入、不同 canonical bytes 和不同 digest。
5. `content_digest` 必须基于 schema 明确定义的 digest projection 计算：根对象移除自身 `content_digest`；带签名的根对象还移除根 `signature`。嵌套对象字段禁止按名称递归删除。签名对象随后必须按 §3.6 重新构造并独立 JCS canonicalize。
6. `undefined`、稀疏数组、循环引用、非字符串对象 key、日期/BigInt/自定义 class 的隐式转换和未声明扩展字段必须在 canonicalization 前拒绝。安全边界必须先完成 strict schema 与 exact-object 校验，再计算 digest；任一失败均禁止执行副作用。
7. 二进制 Artifact 的 digest 必须基于原始字节，不得基于 base64 文本。
8. `fixtures/rfc8785-v1.json` 是 M0 强制 conformance corpus。每种语言实现必须对全部 `valid_cases` 验证 canonical UTF-8 base64 与 SHA-256，对全部 `invalid_cases` 在产生 digest 或签名前拒绝，并把 case id 写入测试报告。该 fixture 自身只使用普通 JSON 表达输入；需要保留重复 key、非法 UTF-8 或超范围 number token 的 invalid case 必须使用 fixture 定义的 encoded source 表达，禁止先交给宽松 JSON parser。
9. 新增或修改 canonicalizer、schema projection、签名库、JSON parser、运行时语言或数据库序列化层时，必须运行该 fixture corpus。跨 Rust/TypeScript 的同一 payload 必须产生完全相同的 canonical bytes 和 digest。

#### 3.2.1 命名 projection registry

所有安全摘要必须使用下列命名 projection，禁止调用方自行选择或递归删除字段：

| Projection | 输入与唯一删除规则 |
| --- | --- |
| `contract_content/v1` | exact root object，删除根 `content_digest`；若根带签名再删除根 `signature` |
| `signed_root/v1` | 已校验且已填入 `content_digest` 的完整 root，删除根 `signature` 后放入 §3.6 signing payload |
| `local_request/v1` | exact request body，删除根 `authentication` 与根 `request_digest`；嵌套 digest/signature 全部保留 |
| `authorization_preimage/v1` | strict Design/Execution admission preimage 的完整 JCS bytes，不删除嵌套字段 |
| `admission_requirements/v1` | exact `AdmissionRequirements` 全对象 |
| `execution_fence/v1` | exact `ExecutionFence` 全对象 |
| `event_set/v1` | 按 cursor 升序的 `{ cursor, event_content_digest }[]`，数组顺序属于摘要 |
| `channel_transcript/v1` | boot evidence、双方 ephemeral public key、nonce、descriptor/fence/boot epoch 和协商算法的 strict transcript |
| `update_activation/v1` | staged receipt、manifest、selection predecessor、schema state、deadline 和 activation nonce 的 strict preimage |
| `revocation_batch/v1` | exact batch header、按 batch 内顺序排列的 strict revocation facts、sequence、previous batch digest、watermark、TTL 与 nonce；删除根 `content_digest`/`signature` 的规则仍由 `contract_content/v1`/`signed_root/v1` 管理 |
| `audit_event_set/v1` | 按 `audit_sequence` 升序的 `{ audit_sequence, audit_event_digest }[]`；数组顺序、首尾 sequence 与前项 digest 均属于摘要 |
| `ledger_integrity/v1` | SQLite schema/transaction high watermark、WAL checkpoint、ledger/audit/outbox/inventory/effect root digest 与 previous checkpoint ref 的 strict preimage |

`request_digest`、`authorization_input_digest`、`admission_requirements_digest`、`fence_digest`、`event_set_digest`、`channel_binding_digest` 和 `activation_request_digest` 必须分别使用上述固定 projection。所有 projection 都必须在共享 corpus 集中提供 canonical UTF-8 base64、digest 和正反例：`contract_content/v1` 与 `signed_root/v1` 由 `fixtures/contract-projection-signing-v1.json` 覆盖；其余 projection 可由该文件或对应的 runtime protocol/vsock/update corpus覆盖，但每个 projection 必须有唯一可追踪 case id，禁止只在文档中声明而无 golden vector。时间统一为 UTC RFC3339 `Z` 表示；禁止时区 offset，fractional second 仅在非零时出现且禁止尾随零。`Sha256` 固定为 `sha256:` 加 64 个 lowercase hex；签名、MAC 和 public key 编码固定为 base64url no-padding。

### 3.3 兼容规则

- `schema_version` 必须使用 `<domain>/<major>` 或 `<domain>/v<major>` 的稳定格式。
- 同一 major 内可以新增 optional 字段；禁止改变既有字段语义、类型或枚举含义。
- 未知 optional 字段应该保留并透传，除非处于安全边界；安全边界必须 fail closed。
- 未知 enum 值必须产生 `contract.unsupported_enum`，禁止静默映射到默认值。
- 不支持的 major 版本必须产生 `contract.unsupported_version`。
- Grant、ApprovalEvidence、PolicyDecision、ActionEnvelope、PlanAction、Runtime reservation/command/event、CommandAdmissionReceipt、VerifierInput/EffectContext、CleanupCapability、InventorySealReceipt、RecoveryDecision、LocalIPCBinding、BootBoundAuthenticatedVsockSession、ExecutableIdentity、ProcessLaunchBinding、CredentialLease、RedactionReceipt 和 Runtime update/release selection 禁止使用“忽略未知字段”的宽松解析。

### 3.4 通用标识

```ts
type ResourceRef = {
  kind: string;
  id: string;
  digest?: Sha256;
  version?: string;
};

type ActorRef = {
  type: "user" | "service" | "device" | "module";
  id: string;
  display_name?: string;
};

type DigestBoundRef<TSchema extends string = string> = {
  kind: string;
  id: string;
  schema_version: TSchema;
  content_digest: Sha256;
  version?: string;
};

type LeaseFence = {
  generation: number;          // 从 1 开始，每次 owner 接管时递增
  fencing_token: string;       // 不透明、不可预测；只比较相等性
};

type ExecutionFence = {
  hosted_workflow: LeaseFence;
  local_execution: LeaseFence;
  runtime_instance_id: string;
};

type RuntimeCursor = {
  generation: number;          // 必须等于 ExecutionFence.local_execution.generation
  sequence: number;            // 0 仅表示该 generation 的 before-first sentinel；持久化 Event 从 1 开始严格递增
};

type SignatureBlock = {
  algorithm: "ed25519" | "es256";
  key_id: string;
  value: Base64UrlNoPad;       // base64url，无 padding
};
```

`DigestBoundRef` 用于所有安全或状态转换绑定；`ResourceRef` 只可用于非权威展示、诊断或兼容数据。接收方禁止仅按 `id` 解析 `DigestBoundRef`，解析结果的 `schema_version` 和 `content_digest` 必须完全匹配。

### 3.5 Strict Discriminated Union

本文标记为 strict union 的类型必须使用判别字段选择唯一 variant，并满足：

1. 未知 discriminator 必须返回 `contract.invalid_variant`，并在安全 details 中记录 `reason=unknown_discriminator`。
2. 缺少该 variant 的 required 字段必须返回 `contract.invalid_variant`，并记录 `reason=missing_required_field`。
3. 出现其他 variant 专属字段必须返回 `contract.forbidden_field`，并记录 `reason=mixed_variant_fields`。
4. 出现 schema 未声明字段必须返回 `contract.forbidden_field`，并记录 `reason=unknown_field`；安全边界禁止沿用 §3.3 的 optional 字段透传规则。
5. validator 必须先选择 variant，再执行该 variant 的 exact-object 校验；禁止把多个 variant 做字段并集后宽松解析。

本文的 `SourceAcquisition`、`DeviceAttestationPurpose`、`DeviceAttestation`、`ApprovalEvidence`、`LocalLeaseBinding`、`GrantClaims`、`DesignPolicyDecision`、`PolicyDecision`、`PlanAction`、`BrowserAction`、`EvidenceRequirement`、`ReserveLocalLeaseRequest`、`AdmissionAuthorizationPreimage`、`AdmissionPredecessor`、`CommandAdmissionReceipt`、`RuntimeCommand`、`ResumeCommand`、`CancellationIntent`、`TimeoutIntent`、`TerminationControlIntent`、`HigherFenceCleanupAuthority`、`CleanupCommand`、`RuntimeEventCause`、`RuntimeEvent`、`RuntimeStreamPosition`、`RuntimePairingReceipt`、`LocalIPCBinding`、`RevocationFact`、`RevocationDeliveryReceipt`、`VerifierInput`、`EffectRequest`、`EffectAuthorization`、`EffectContext`、`CheckedDigests`、`EffectDecision`、`EffectReceipt`、`PrepareDesignResult`、`PrepareExecutionResult`、`VZSandboxReceipt`、`DependencyAcquisitionReceipt`、`ResourceLimitReceipt`、`NetworkFlowReceipt`、`SecretBrokerRequest`、`SecretMaterializationReceipt`、`InventorySealReceipt`、`CleanupCapability`、`LocalExecutionLease`、`TerminationTargetScope`、`TerminationReceipt`、`RedactionRule`、`RedactionReceipt`、`CleanupReceipt`、`RecoveryDecision`、`AuditSubject`、`AuditEvent`、`LedgerIntegrityVerificationReceipt`、`SafeErrorDetails`、`QualityEvaluation`、`ProjectPackPromotionReceipt` 和 `RuntimeUpdateReceipt` 均为 strict union。每个 strict union variant 都禁止出现其他 variant 的专属字段；本文后续省略该句时仍适用 §3.5 的 exact-object 与 forbidden cross-variant field 规则。

### 3.6 Canonical Signing Payload

签名对象必须先按 §3.2 计算 payload 自身的 `content_digest`，再签名。签名字节必须是以下对象的 canonical JSON UTF-8 bytes，禁止签名 pretty JSON、base64 文本、仅 digest 字符串或传输层 envelope：

```ts
type CanonicalSigningPayload<T> = {
  domain: "fkst.qa.signature/v1";
  purpose:
    | "approval_evidence"
    | "device_attestation"
    | "grant"
    | "grant_revocation_snapshot"
    | "revocation_batch"
    | "revocation_delivery_receipt"
    | "local_lease_binding"
    | "source_object_lease"
    | "cleanup_capability"
    | "credential_receipt"
    | "secret_materialization_receipt"
    | "inventory_seal_receipt"
    | "cancellation_intent"
    | "timeout_intent"
    | "fence_transition_authorization"
    | "grant_revocation_receipt"
    | "recovery_decision"
    | "local_ipc_binding"
    | "local_ipc_request"
    | "local_ipc_response"
    | "runtime_identity_statement"
    | "runtime_pairing_challenge"
    | "runtime_pairing_receipt"
    | "artifact_access_capability"
    | "boot_bound_authenticated_vsock_session"
    | "guest_boot_evidence"
    | "secret_broker_binding"
    | "secret_broker_health"
    | "secret_broker_receipt"
    | "browser_enforcement_evidence"
    | "runtime_hard_ceilings"
    | "resource_limit_binding"
    | "dependency_acquisition_receipt"
    | "resource_limit_receipt"
    | "network_flow_receipt"
    | "redaction_receipt"
    | "runtime_degraded_operation_matrix"
    | "runtime_admission_snapshot"
    | "runtime_run_snapshot"
    | "command_admission_receipt"
    | "runtime_event_batch"
    | "runtime_repair_receipt"
    | "repair_operation"
    | "runtime_health_evidence"
    | "runtime_activation_request"
    | "runtime_activation_result"
    | "runtime_update_receipt"
    | "audit_event"
    | "audit_checkpoint"
    | "ledger_integrity_checkpoint"
    | "ledger_integrity_verification_receipt"
    | "verification_waiver"
    | "verification_gate_result"
    | "runtime_release_selection"
    | "runtime_update_manifest";
  key_id: string;
  payload: T;                  // 完整 payload，包含已校验的 content_digest，不含 SignatureBlock
};
```

签名验证方必须重建该对象并验证 `key_id`、`purpose`、payload schema、payload digest 和签名。任何字段缺失、额外字段、非法 UTF-8、lone surrogate、重复 member name、数值约束失败或 digest 不一致均必须拒绝。验证方禁止在验签前后对 Unicode 做 normalization；发送方与接收方必须直接对同一 JCS bytes 达成一致。

---

## 4. `RunDraft`、`SourceAcquisition`、`SourceRevision` 与 `RunSpec`

### 4.1 RunDraft

```ts
type RunDraft = ContractMeta & {
  draft_id: string;
  draft_version: number;
  trigger: {
    type: "pull_request" | "manual" | "api" | "scheduled" | "rerun";
    actor: ActorRef;
    parent_run_id?: UUID;
  };
  source_request:
    | { kind: "pull_request"; provider: "github"; owner: string; name: string; number: number }
    | { kind: "revision"; provider: "github"; owner: string; name: string; revision_hint: string };
  project: {
    project_id: string;
    profile_id: string;
    profile_digest: Sha256;
    pql_project_pack?: DigestBoundRef<"qa.pql-project-pack/v1">;
  };
  requested_scope: {
    suites?: string[];
    case_ids?: string[];
    paths?: string[];
    tags?: string[];
    max_duration_seconds: number;
  };
  target_device: {
    selector: { type: "device_id"; value: string } | { type: "capability"; value: string };
    required_capabilities: string[];
    platform: "macos";
  };
  policy: DigestBoundRef<"qa.policy/v1">;
  publication_intent: {
    github_check: boolean;
    pr_comment: boolean;
    product_issue: "disabled" | "quality_gate_only";
    pql_feedback: boolean;
  };
};
```

`RunDraft` 每个版本不可变，但在 Source 获取成功前可以创建新 draft version。Draft 禁止被 Authorization Authority、Runtime 或 testing-design 当作可执行输入。

### 4.2 SourceRevision

```ts
type SourceObject = {
  object_ref: DigestBoundRef<"qa.source-object/v1">;
  object_digest: Sha256;            // Git bundle/pack 或等价不可变对象的原始字节 digest
  object_format: "git_bundle" | "git_pack" | "content_addressed_snapshot";
  retention: {
    policy_id: string;
    retain_until: ISO8601;
    audit_hold: boolean;
  };
  resolver: {
    id: "hosted-source-resolver";
    version: string;
    resolved_at: ISO8601;
  };
};

type SourceObjectLease = ContractMeta & {
  lease_id: string;
  issuer: "fkst-hosted.source-resolver";
  audience: "fkst-local-qa-runtime";
  source_object_ref: DigestBoundRef<"qa.source-object/v1">;
  source_object_digest: Sha256;
  device_id: string;
  runtime_instance_id: string;
  operation: "download_once";
  max_bytes: number;
  not_before: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type SourceRevision =
  | {
      kind: "pull_request";
      repository: {
        provider: "github";
        owner: string;
        name: string;
        repository_id: string;
      };
      pull_request: {
        number: number;
        base_sha: string;
        head_sha: string;
        head_repository?: string;
        is_fork: boolean;
      };
      resolution: {
        strategy: "synthetic_merge_commit";
        effective_sha: string;
        merge_tree_digest: Sha256;
        created_by: "hosted-source-resolver";
        resolver_version: string;
        created_at: ISO8601;
      };
      source_object: SourceObject;
    }
  | {
      kind: "commit";
      repository: {
        provider: "github";
        owner: string;
        name: string;
        repository_id: string;
      };
      resolution: {
        strategy: "exact_commit";
        effective_sha: string;
      };
      source_object: SourceObject;
    };
```

PR Run 必须满足：

- `effective_sha` 必须指向 hosted source resolver 生成并持久记录的 synthetic merge commit。
- synthetic merge 必须由固定的 `base_sha` 和 `head_sha` 生成；禁止在执行时重新解析浮动分支。
- hosted source resolver 必须通过不可变的受控 ref、Git bundle 或等价对象存储保存 synthetic commit 及其可达对象，保留时间至少覆盖 Run、重试、审计和 Artifact retention policy 要求。
- Local Runtime 必须从受控来源获取该对象并校验 `effective_sha`、`merge_tree_digest` 和 `source_object.object_digest`；禁止在设备端重新生成另一个 synthetic merge。
- merge conflict 必须在授权前产生 `SourceAcquisition(kind="blocked")`，reason code 为 `source.merge_conflict`，不得回退为只测 head SHA。
- GitHub 临时 merge ref 可以用于对照，但禁止作为唯一可重放依据。

非 PR Run 必须满足：

- `effective_sha` 必须是 exact commit SHA。
- branch、tag 或默认分支名称只可作为输入提示，必须在 SourceAcquisition 时解析并冻结为 SHA。

`clone_url`、repository web URL、用户输入的 remote URL 和 Git 配置均不是授权来源。Source resolver 和 Local Runtime 只能通过受信任的 repository identity 映射、`SourceObject.object_ref`、显式 transport policy 和短期 `SourceObjectLease` 获取源码；任何 URL 最多作为非权威显示信息。

`SourceObjectLease` 是 hosted source resolver 对已冻结 SourceObject 的一次性下载能力，不是 Design Secret，也不能访问任意 repository。它必须按 §3.6 的 `purpose="source_object_lease"` 签名，并绑定 object ref/digest、目标 device/runtime、最大字节数、TTL 和 nonce。Rust Supervisor 验证并消费 lease，把已校验 object 导入 Design VM；TypeScript worker、NyxID 和 guest 禁止取得可复用下载 credential。Lease 过期或 digest 不匹配必须阻止 Design，禁止回退到 clone 或浮动 ref。

### 4.3 SourceAcquisition

```ts
type SourceAcquisition =
  | (ContractMeta & {
      kind: "acquired";
      acquisition_id: string;
      run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
      source_revision: SourceRevision;
      acquired_at: ISO8601;
      resolver_attestation_digest: Sha256;
    })
  | (ContractMeta & {
      kind: "blocked";
      acquisition_id: string;
      run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
      reason_code: "source.merge_conflict" | "source.policy_blocked" | "source.revision_not_found";
      observed_refs: ResourceRef[];
      blocked_at: ISO8601;
    })
  | (ContractMeta & {
      kind: "failed";
      acquisition_id: string;
      run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
      error: ErrorEnvelope;
      retryable: boolean;
      failed_at: ISO8601;
    });
```

只有 `kind="acquired"` 的 `SourceAcquisition` 可以用于冻结 `RunSpec`。`blocked` 表示输入或策略确定性阻断，`failed` 表示获取过程失败；两者均必须进入 non-executed QualityEvaluation，禁止伪造空 `SourceRevision`。

### 4.4 RunSpec

```ts
type RunSpec = ContractMeta & {
  run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source: SourceRevision;
  project: {
    project_id: string;
    profile_id: string;
    profile_ref: DigestBoundRef<"qa.project-profile/v1">;
    profile_digest: Sha256; // 仅供索引/快速比较；必须等于 profile_ref.content_digest，不能单独授权
    pql_project_pack?: DigestBoundRef<"qa.pql-project-pack/v1">;
  };
  requested_scope: {
    suites?: string[];
    case_ids?: string[];
    paths?: string[];
    tags?: string[];
    max_duration_seconds: number;
  };
  target_device: {
    selector: { type: "device_id"; value: string } | { type: "capability"; value: string };
    required_capabilities: string[];
    platform: "macos";
  };
  policy: DigestBoundRef<"qa.policy/v1">;
  publication_intent: {
    github_check: boolean;
    pr_comment: boolean;
    product_issue: "disabled" | "quality_gate_only";
    pql_feedback: boolean;
  };
};
```

`RunSpec` 必须在 SourceAcquisition 成功后一次性冻结。冻结时必须验证 `run_draft_ref`、`source_acquisition_ref`、`source.source_object`、project profile 和 policy 的 digest 绑定；允许变化的后续数据必须通过独立版本对象表达，禁止原地修改 `RunSpec`。

---

## 5. `ApprovalEvidence` 与 `GrantClaims`

### 5.1 ApprovalEvidence

```ts
type ExternalApprovalProvider = {
  type: "nyxid" | "enterprise_device_agent" | "local_cli";
  provider_id: string;
  provider_version: string;
};

type DeviceAttestationBase = ContractMeta & {
  attestation_id: string;
  provider: ExternalApprovalProvider;
  user: ActorRef & { type: "user" };
  device: {
    device_id: string;
    platform: "macos";
    platform_version: string;
    hardware_model: string;
    device_key_id: string;
  };
  challenge_id: string;
  challenge_nonce: string;
  attestation_request_ref: DigestBoundRef<"qa.device-attestation-request/v1">;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_instance_id: string;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  assurance: {
    level: "software_bound" | "hardware_backed" | "managed_device";
    user_presence: "verified" | "not_verified";
    secure_key_storage: "keychain" | "secure_enclave" | "provider_managed";
    device_posture_ref?: DigestBoundRef<"qa.device-posture/v1">;
  };
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type DeviceAttestationPurpose =
  | {
      purpose: "design_approval" | "execution_approval";
      runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
    }
  | {
      purpose: "runtime_pairing";
      requested_pairing_epoch: number;
      pairing_receipt_absent: true;
    };

type DeviceAttestation = DeviceAttestationBase & DeviceAttestationPurpose & (
  | { status: "valid" }
  | { status: "revoked"; revoked_at: ISO8601; revocation_reason: "device_removed" | "identity_rotated" | "pairing_revoked" | "provider_security_action" }
);

type ApprovalSubject = {
  user: ActorRef & { type: "user" };
  device_id: string;
  device_attestation_ref: DigestBoundRef<"qa.device-attestation/v1">;
};

type DesignApprovalEvidence = ContractMeta & {
  kind: "design";
  approval_id: string;
  issuer:
    | ExternalApprovalProvider
    | { type: "hosted_policy"; provider_id: "fkst-hosted.authorization-authority"; provider_version: string };
  subject: ApprovalSubject;
  decision: "approved" | "denied";
  evidence_basis: "user_approval" | "policy_not_required";
  binding: {
    run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
    design_policy_decision_ref: DigestBoundRef<"qa.design-policy-decision/v1">;
    source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
    source_object_ref: DigestBoundRef<"qa.source-object/v1">;
    source_effective_sha: string;
    design_scope_digest: Sha256;
    project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  };
  decided_at: ISO8601;
  expires_at: ISO8601;
  challenge_nonce: string;
  signature: SignatureBlock;
};

type ExecutionApprovalEvidence = ContractMeta & {
  kind: "execution";
  approval_id: string;
  issuer:
    | ExternalApprovalProvider
    | { type: "hosted_policy"; provider_id: "fkst-hosted.authorization-authority"; provider_version: string };
  subject: ApprovalSubject;
  decision: "approved" | "denied";
  evidence_basis: "user_approval" | "policy_not_required";
  binding: {
    run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
    source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
    source_effective_sha: string;
    plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
    policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
    approved_envelope_digest: Sha256;
    device_id: string;
    project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
    grant_sequence: number;
  };
  decided_at: ISO8601;
  expires_at: ISO8601;
  challenge_nonce: string;
  signature: SignatureBlock;
};

type ApprovalEvidence = DesignApprovalEvidence | ExecutionApprovalEvidence;
```

`ApprovalEvidence` 是 strict union。Design variant 禁止出现 `plan_ref`、`policy_decision_ref`、`approved_envelope_digest` 或 Secret 字段；Execution variant 缺少任一 plan/policy/envelope/device/profile/sequence 绑定时无效。

NyxID Adapter 必须把用户批准和设备证明映射为对应 variant。`evidence_basis="policy_not_required"` 只能由 hosted Authorization Authority 在对应 DesignPolicyDecision 或 PolicyDecision 的 `approval_requirement.kind="not_required"` 时签发，仍必须生成不可变的对应 ApprovalEvidence，禁止用空数组表示“无需 Evidence”。Authorization Authority 必须验证签名、challenge nonce、device binding、有效期、Run 绑定和全部 digest。

`DeviceAttestation` 必须由 `provider` 的受信任 attestation key 按 §3.6 `purpose="device_attestation"` 签名。验签方必须把 `attestation_request_ref` 解析为本次 challenge 的 exact request，逐项比较 user/device、`run_id`、purpose、challenge id/nonce、Runtime identity 与两个 epoch；design/execution purpose 必须绑定当前 active pairing receipt，runtime_pairing purpose 必须声明 requested epoch 并明确 pairing receipt 尚不存在。所有消费方都必须要求 `status="valid"`、`issued_at <= decided_at < expires_at`。attestation 只证明 provider 声明的 assurance，不得把 `software_bound` 冒充硬件证明；Runtime identity rotation、re-pair、pairing revocation、device removal 或 provider security revocation 后，旧 attestation 即使 TTL 未过也不得用于新 Evidence。ApprovalEvidence 与 Grant 必须保存 `device_attestation_ref`，禁止退化为无法解析、无法检查 schema/issuer/expiry/revocation 的裸 digest。Project profile 在 RunSpec、Plan、Approval、admission preimage 和 Grant 中必须使用 `project_profile_ref`；RunSpec 保留的 `profile_digest` 只可作为与 `profile_ref.content_digest` 强制相等的索引字段。`design_scope_digest` 与 `approved_envelope_digest` 可以保留，因为同一对象同时以内嵌 `DesignScope`/`ActionEnvelope` 和对应 PolicyDecision ref 提供完整可解析 authority，digest 仅用于固定 projection 的快速一致性比较，禁止单独作为授权句柄。

`decision="denied"` 的 Evidence 必须保留用于审计，但禁止出现在任何 `approval_evidence_refs`、Grant、RuntimeCommand、PlanAmendment 或 ResumeDirective 中。签名字节必须按 §3.6 构造，其中 `purpose="approval_evidence"`，`payload` 为移除 `signature` 后的完整 Evidence。

### 5.2 GrantClaims

```ts
type LocalLeaseBindingBase = ContractMeta & {
  binding_id: string;
  device_id: string;
  runtime_instance_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  hosted_workflow: LeaseFence;
  reserved_local_execution: LeaseFence;
  predecessor_active_local_execution?: LeaseFence;
  authorization_input_digest: Sha256;
  admission_requirements_digest: Sha256;
  observed_admission_snapshot_ref: DigestBoundRef<"qa.runtime-admission-snapshot/v1">;
  observed_admission_snapshot_digest: Sha256;
  observed_admission_snapshot_expires_at: ISO8601;
  reservation_epoch: string;
  quota_hold_digest: Sha256;
  runtime_capability_digest: Sha256;
  runtime_version: string;
  compatibility_set_digest: Sha256;
  guest_image_digest: Sha256;
  observed_disk_pressure: "normal" | "warning";
  reservation_request_digest: Sha256;
  reservation_nonce: string;
  reservation_idempotency_key: string;
  reserved_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type LocalLeaseBinding = LocalLeaseBindingBase & (
  | {
      phase: "design" | "amendment_design";
      authorization_preimage: DesignAdmissionAuthorizationPreimage;
    }
  | {
      phase: "execution" | "amendment_execution";
      authorization_preimage: ExecutionAdmissionAuthorizationPreimage;
    }
);

type GrantTiming = {
  issued_at: ISO8601;
  not_before: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  sequence: number;
};

type GrantRevocation = {
  revocable: true;
  snapshot_endpoint_ref: DigestBoundRef<"qa.grant-revocation-feed/v1">;
  delivery_feed_ref: DigestBoundRef<"qa.revocation-feed/v1">;
  snapshot_issuer: "fkst-hosted.authorization-authority";
  max_snapshot_age_seconds: number;
  max_delivery_age_seconds: number;
  offline_behavior: "quiesce_non_cleanup_effects";
};

type GrantRevocationSnapshot = ContractMeta & {
  snapshot_id: string;
  issuer: "fkst-hosted.authorization-authority";
  audience: "fkst-local-qa-runtime";
  device_id: string;
  runtime_instance_id: string;
  sequence_high_watermark: number;
  grants: Array<{
    grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
    grant_sequence: number;
    status: "active" | "revoked" | "expired" | "superseded";
    revoked_at?: ISO8601;
    reason?: "cancelled" | "timed_out" | "amendment" | "security" | "superseded";
  }>;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type GrantDeviceBinding = {
  device_id: string;
  runtime_instance_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  device_attestation_ref: DigestBoundRef<"qa.device-attestation/v1">;
};

type DesignScope = {
  source_read_roots: RootQualifiedPathPattern[];
  metadata_read_roots: RootQualifiedPathPattern[];
  pql_input_refs: DigestBoundRef[];
  workspace_write_roots: RootQualifiedPathPattern[];
  static_analysis_tools: string[];
  network: ActionEnvelope["network"];
  resources: ActionEnvelope["resources"];
  capabilities: Array<"source.read" | "metadata.read" | "plan.write" | "static_analysis.execute">;
};

type DesignGrantClaims = ContractMeta & {
  grant_type: "design";
  grant_id: string;
  issuer: "fkst-hosted.authorization-authority";
  subject: { run_id: UUID; device_id: string; runtime_instance_id: string };
  audience: "fkst-local-qa-runtime";
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source_object_ref: DigestBoundRef<"qa.source-object/v1">;
  source_effective_sha: string;
  project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  design_policy_decision_ref: DigestBoundRef<"qa.design-policy-decision/v1">;
  authorized_fence: ExecutionFence;
  local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  authorization_input_digest: Sha256;
  admission_requirements_digest: Sha256;
  design_scope: DesignScope;
  design_scope_digest: Sha256;
  device_binding: GrantDeviceBinding;
  approval_evidence_refs: [DigestBoundRef<"qa.approval-evidence/v1">, ...DigestBoundRef<"qa.approval-evidence/v1">[]];
  timing: GrantTiming;
  revocation: GrantRevocation;
};

type ExecutionGrantClaims = ContractMeta & {
  grant_type: "execution";
  grant_id: string;
  issuer: "fkst-hosted.authorization-authority";
  subject: { run_id: UUID; device_id: string; runtime_instance_id: string };
  audience: "fkst-local-qa-runtime";
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source_effective_sha: string;
  project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  authorized_fence: ExecutionFence;
  local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  authorization_input_digest: Sha256;
  admission_requirements_digest: Sha256;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
  approved_envelope: ActionEnvelope;
  approved_envelope_digest: Sha256;
  scope: ActionEnvelope;
  device_binding: GrantDeviceBinding;
  approval_evidence_refs: [DigestBoundRef<"qa.approval-evidence/v1">, ...DigestBoundRef<"qa.approval-evidence/v1">[]];
  timing: GrantTiming;
  revocation: GrantRevocation;
};

type GrantClaims = DesignGrantClaims | ExecutionGrantClaims;

type SignedGrant<TClaims extends GrantClaims = GrantClaims> = {
  claims: TClaims;
  signature: SignatureBlock;
};
```

`GrantClaims` 是 strict union，两个 variant 的 `issuer` 都必须是 `fkst-hosted.authorization-authority`，并绑定目标 runtime 的 `authorized_fence` 和 `local_lease_binding_ref`。`authorized_fence.hosted_workflow` 必须等于 binding 的 `hosted_workflow`，`authorized_fence.local_execution` 必须等于 `reserved_local_execution`；subject device/runtime、Runtime identity/pairing refs/epochs 以及 Grant 中的 Source、RunSpec、Plan、Policy、envelope、profile 和 scope 字段必须与 `LocalLeaseBinding.authorization_preimage` 的对应字段逐项相等，且其完整 JCS digest 必须等于 binding 的 `authorization_input_digest`。Design Grant 只能消费 design/amendment_design variant，Execution Grant 只能消费 execution/amendment_execution variant。Design variant 禁止出现 plan、PolicyDecision、approved envelope、Execution scope 或任何 Secret reference；Execution variant 必须绑定 plan、PolicyDecision、批准 envelope、device/profile、ExecutionFence、LocalLeaseBinding 和严格递增 sequence。

`LocalLeaseBinding` 必须由目标 Rust Runtime 的 device-bound signing key 签名，签名字节按 §3.6 构造，其中 `purpose="local_lease_binding"`。Reservation 只分配候选 generation/token 并证明目标 Runtime 愿意在有效期内接收相应 Grant；它禁止修改 active `LocalExecutionLease`、禁止 fence 当前 generation、禁止启动资源或消费 Grant nonce。过期、被取消或未被命令激活的 reservation 必须可直接回收且不得影响现有执行。

Grant 签名字节必须按 §3.6 构造，其中 `purpose="grant"`，`payload=claims`。Runtime 必须拒绝 claims/signature key 不匹配、非 canonical payload、未知字段、混合 variant 字段、binding digest/signature 不匹配或 reservation 已过期/已消费。

### 5.3 Design Grant

Design Grant 的 `design_scope` 只允许读取固定 SourceObject、批准的 PQL 资产和项目元数据，执行静态分析，并在 Runtime 管理的临时目录写入 Structured Plan 草稿。它必须禁止：

- 绑定或读取 Plan、PolicyDecision approved envelope、CredentialLease 或 Secret reference。
- 启动 App、Middleware、浏览器、测试、发布动作或长期进程。
- 修改被测仓库内容。
- 对公网或内网发起未在 `design_scope.network` 中显式批准的连接。

### 5.4 Execution Grant

Execution Grant 的 `scope` 必须等于或小于 `approved_envelope`，且两者的 canonical digest 必须与 PolicyDecision 一致。Runtime 必须拒绝以下情况：

- Grant 已过期、尚未生效、已撤销或 nonce 已使用。
- effective SHA、RunSpec、SourceAcquisition、Plan、PolicyDecision、approved envelope、device、attestation 或 project profile ref 不匹配。
- 任一 ApprovalEvidence 为 denied、过期、类型非 execution、sequence 不匹配或 digest 不匹配。
- signature key 不受信任，或 sequence 小于等于已接受的最新 Execution Grant sequence。

Runtime 对 Design/Execution Grant 的第一次命令 admission 必须在单个 SQLite transaction 内完成。事务开始后必须再次验证 LocalLeaseBinding/Grant/preimage，确认 reservation 尚未过期或消费，并比较 `CommandPrecondition`、`CommandTarget` 与 command sequence；随后原子创建由 `(run_id, phase, reserved_local_execution.generation)` 确定且重试稳定的 `environment_id`、version=1/state=open/resources=[] 的空 inventory lineage root、覆盖该 environment/lineage/actions/reasons/TTL/nonce 且已签名的完整 CleanupCapability、active LocalExecutionLease、FenceTransition、strict `AdmissionPredecessor`、CommandAdmissionReceipt、初始 `EffectRecord(state="pending")` 集合和按 cursor 排序的 Event outbox。initial variant 禁止创建或引用伪造的 PredecessorFencingRecord；takeover variant 必须绑定真实 predecessor lease/fence/cursor/inventory。第一项 outbox event 必须是 sequence=1 的 `command_accepted`，同一事务中的后续 activation/inventory/capability event 依次递增。事务还必须记录 Grant nonce/sequence；Execution Grant 额外记录 `(grant_id, timing.nonce, timing.sequence, plan_ref.content_digest)`。事务提交前禁止启动 VM、进程、端口、目录、mount、Browser 或 Secret；事务失败时不得留下 environment、inventory、capability、lease、fence、nonce、cursor 或 outbox 的部分写入，active generation 必须保持不变。提交后所有 bootstrap 副作用必须使用不依赖 PlanStep 或 PreparedEnvironment 的 phase-specific bootstrap EffectContext。

---

## 6. `StructuredPlan`、`PlanCase`、`PlanStep`、`ActionEnvelope` 与 Policy

### 6.1 ActionEnvelope

```ts
type SandboxRootName = "source" | "workspace" | "artifact_staging" | "runtime_metadata";

type RootQualifiedPath = {
  root: SandboxRootName;
  relative_path: string;            // root 内 POSIX relative path；禁止前导 `/`、`.`、`..`、空 segment、NUL 和 `\\`
};

type RootQualifiedPathPattern = {
  root: SandboxRootName;
  relative_pattern: string;         // root 内 anchored POSIX glob
};

type ActionEnvelope = {
  files: {
    read: RootQualifiedPathPattern[];
    write: RootQualifiedPathPattern[];
    deny: RootQualifiedPathPattern[];
  };
  commands: Array<{
    executable: string;
    argv_template: Array<
      | { kind: "literal"; value: string }
      | { kind: "enum"; values: string[] }
      | { kind: "sandbox_path"; roots: SandboxRootName[]; access: "read" | "write" }
    >;
    working_directory_roots: SandboxRootName[];
    allow_shell_expansion: false;
  }>;
  network: {
    mode: "deny_all" | "allowlist";
    destinations: Array<{
      scheme: "https" | "http" | "tcp";
      host: string;
      ports: number[];
      purpose: string;
    }>;
  };
  secrets: Array<{
    secret_ref: string;
    inject_as: "environment" | "file" | "proxy";
    step_ids: string[];
    allowed_destinations?: string[];
  }>;
  resources: {
    cpu_millis: number;
    memory_bytes: number;
    disk_bytes: number;
    process_count: number;
    open_file_count: number;
    wall_clock_seconds: number;
  };
  capabilities: string[];
};
```

所有路径契约必须 root-qualified；裸绝对路径、宿主路径、仅靠字符串前缀表达的“sandbox path”和未声明 root 均非法。Runtime 必须在环境激活时把每个 `SandboxRootName` 绑定到一个 immutable root identity，并对每次访问先解析 root identity、再逐 segment 以 no-follow 语义解析 `relative_path`。`..`、symlink/mount escape、case-folding alias、root 替换和用户主目录路径必须拒绝。`deny` 优先于 `read` 和 `write`。文件 pattern 使用 qa-contracts 定义的 anchored POSIX glob：`*` 不跨 `/`、`**` 才可跨目录；匹配只使用 schema 指定的 code point 和 `/` 分隔语义，禁止隐式 Unicode normalization 或跟随 symlink 越出 root。

v1 命令参数只允许 `argv_template` 的 literal、有限 enum 和已批准 sandbox path token，禁止 regex、任意 glob、shell 字符串和调用方自定义 matcher；`allow_shell_expansion` 必须恒为 `false`。需要管道、重定向或变量展开时必须把它建模成显式多个 EffectRequest，禁止交给 `/bin/sh -c`。

网络 `host` 必须是 canonical ASCII DNS name 或显式 IP literal，禁止 wildcard、URL userinfo 和调用方控制的解析器。连接必须经 EffectGate egress adapter 解析并在 connect 时校验全部 A/AAAA 结果；DNS rebinding、解析到未批准 private/link-local/metadata 地址、CNAME 跳转到 envelope 外目标都必须拒绝。若策略确需私网目标，必须在 destination 中显式列出对应 IP/CIDR capability，不能只批准一个可重绑定域名。

### 6.2 PlanCase、PlanStep、Assertion 与 EvidenceRequirement

```ts
type AssertionBase = {
  assertion_id: string;
  required: boolean;
};

type AssertionSpec = AssertionBase & (
  | { type: "exit_code"; expected: number }
  | { type: "stdout_match"; pattern: string; flags?: string }
  | { type: "http_status"; request_ref: string; expected: number[] }
  | { type: "json_schema"; source_ref: string; schema_ref: string }
  | { type: "dom"; selector: string; operator: "exists" | "text_equals" | "attribute_equals"; expected?: string; attribute?: string }
  | { type: "visual"; baseline_ref: string; max_diff_ratio: number }
  | { type: "custom_oracle"; oracle_id: string; input_refs: string[] }
);

type EvidenceRequirementBase = {
  requirement_id: string;
  required: boolean;
  step_ids: string[];
  case_ids: string[];
  assertion_ids: string[];
  redaction_policy_ref: DigestBoundRef<"qa.redaction-policy/v1">;
};

type EvidenceRequirement = EvidenceRequirementBase & (
  | { type: "log"; stream: "stdout" | "stderr" | "combined"; max_bytes: number }
  | { type: "screenshot"; capture: "on_assertion" | "on_failure" | "always"; full_page: boolean }
  | { type: "trace"; format: "playwright" | "browser_devtools" | "custom" }
  | { type: "video"; max_duration_seconds: number }
  | { type: "network"; mode: "metadata_only" | "redacted_payload" }
  | { type: "structured_observation"; schema_ref: DigestBoundRef }
  | { type: "report"; renderer_id: string }
);

type ApplicabilityCondition =
  | { kind: "always" }
  | { kind: "runtime_capability"; capability: string; present: boolean }
  | { kind: "project_profile_value"; json_pointer: string; operator: "equals" | "in"; value: unknown }
  | { kind: "prior_step_outcome"; step_id: string; outcomes: Array<"passed" | "failed" | "error" | "skipped"> };

type BrowserAction =
  | { kind: "browser_launch"; start_url?: string; required_capabilities: BrowserSecurityCapability[] }
  | { kind: "browser_navigate"; session_ref: DigestBoundRef<"qa.browser-session/v1">; url: string; wait_until: "load" | "domcontentloaded" | "networkidle" }
  | { kind: "browser_click"; session_ref: DigestBoundRef<"qa.browser-session/v1">; selector: string; button: "left" | "middle" | "right" }
  | { kind: "browser_fill"; session_ref: DigestBoundRef<"qa.browser-session/v1">; selector: string; value_ref: DigestBoundRef<"qa.nonsecret-input/v1"> }
  | { kind: "browser_select"; session_ref: DigestBoundRef<"qa.browser-session/v1">; selector: string; values: string[] }
  | { kind: "browser_capture"; session_ref: DigestBoundRef<"qa.browser-session/v1">; requirement_ref: DigestBoundRef<"qa.evidence-requirement/v1">; capture: "screenshot" | "dom" | "trace" | "network" }
  | { kind: "browser_terminate"; session_ref: DigestBoundRef<"qa.browser-session/v1">; reason: TerminationReason };

type PlanAction =
  | { kind: "file_read"; path: RootQualifiedPath; max_bytes: number }
  | { kind: "file_write"; path: RootQualifiedPath; content_ref: DigestBoundRef<"qa.action-content/v1">; create: "new" | "replace" }
  | { kind: "file_delete"; path: RootQualifiedPath }
  | {
      kind: "process_run";
      executable: string;
      argv: Array<{ kind: "literal"; value: string } | { kind: "sandbox_path"; value: RootQualifiedPath }>;
      working_directory: RootQualifiedPath;
      expected_executable_digest?: Sha256;
    }
  | { kind: "network_request"; scheme: "https" | "http" | "tcp"; host: string; port: number; method?: string; request_ref?: DigestBoundRef<"qa.network-request/v1"> }
  | BrowserAction
  | { kind: "readiness_probe"; check: ReadinessCheck }
  | { kind: "service_stop"; target_process_ref: DigestBoundRef<"qa.process-identity/v1">; reason: TerminationReason }
  | { kind: "artifact_stage"; source: RootQualifiedPath; media_type: string; evidence_requirement_ids: string[] };

type PlanStep = {
  step_id: string;
  ordinal: number;
  name: string;
  purpose: string;
  phase: "prepare" | "readiness" | "execute" | "evidence" | "cleanup";
  backend: "deterministic" | "browser" | "codex";
  dependencies: string[];
  case_ids: string[];
  applicability: ApplicabilityCondition;
  action: PlanAction;
  envelope: ActionEnvelope;
  assertions: AssertionSpec[];
  evidence_requirement_ids: string[];
  timeout_seconds: number;
  retry: { max_attempts: number; retry_on: string[]; backoff_seconds: number[] };
  continue_on_failure: boolean;
};

type PlanCase = {
  case_id: string;
  suite_id?: string;
  name: string;
  required: boolean;
  applicability: ApplicabilityCondition;
  step_ids: string[];
  assertion_ids: string[];
  evidence_requirement_ids: string[];
  aggregation: "all_required_assertions" | "any_required_assertion";
};
```

`PlanAction`、`BrowserAction` 和 `EvidenceRequirement` 是 strict union。每个 variant 必须拒绝其他动作的字段，例如 `browser_launch` 禁止携带 `session_ref`，非 launch BrowserAction 必须携带 session，非 BrowserAction 禁止携带 selector/session/capture 字段。每个 `assertion_id`、`requirement_id`、`step_id` 和 `case_id` 在 Plan 内必须唯一；所有引用必须存在且双向一致：Case 引用 Step 时 Step 必须包含该 Case，Case 引用 Assertion 时该 Assertion 必须属于其 Step，EvidenceRequirement 的 Case/Step/Assertion 绑定也必须一致。

`ApplicabilityCondition` 只能读取 immutable RunSpec/project profile、Runtime capability snapshot 或已 settled 的 prior Step outcome；禁止读取当前时间、随机数、网络、Secret、可变宿主状态或执行任意表达式。Runner 必须记录 condition 输入摘要和计算结果。Case/Step 只有在 condition 明确为 false 时才可按 Plan 跳过；condition 无法求值必须产生 error/inconclusive，禁止当作 false。

Case 聚合规则如下：

1. 任一 required assertion 为 failed 时 Case 为 failed。
2. 任一 required assertion 为 error 或无法评估，且没有策略允许的等价 fallback 时 Case 为 error 或 inconclusive，禁止 passed。
3. `all_required_assertions` 只有全部 required assertion passed 才可 passed；`any_required_assertion` 至少一个 required assertion passed 且其余 required assertion 不为 failed/error 才可 passed。
4. required EvidenceRequirement 未 fulfilled 时，Case 即使 assertion passed 也只能为 inconclusive；optional Evidence 缺失不得改变 assertion outcome，但必须记录。
5. Step outcome 由其 AssertionResult 聚合，Case outcome 由其全部 Step/Assertion/Evidence 聚合；Plan 汇总禁止只读取最后一个 Step。

Browser Readiness 只能在 Plan 包含 Browser Step 时出现。API、CLI、unit-only Plan 不应该启动浏览器或 Browser Profile。

Codex Step 必须满足：

- 输入和动作受 `ActionEnvelope` 约束。
- 输出必须是观察值、建议或 `amendment_required`，不得直接改写 Plan。
- Codex 自报 `passed: true` 禁止覆盖 assertion evaluator 的结果。
- 未产生所需结构化输出时必须视为 Backend protocol failure。

### 6.3 StructuredPlan

```ts
type StructuredPlan = ContractMeta & {
  plan_id: string;
  version: number;
  previous_plan_ref?: DigestBoundRef<"qa.structured-plan/v1">;
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source_effective_sha: string;
  project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  pql_inputs: DigestBoundRef[];
  cases: PlanCase[];
  steps: PlanStep[];
  evidence_requirements: EvidenceRequirement[];
  aggregate_envelope: ActionEnvelope;
  assertion_catalog_digest: Sha256;
  cleanup_requirements: string[];
  generated_by: { module: "testing-design"; model_or_engine?: string };
};
```

`aggregate_envelope` 必须是所有 Step envelope 的最小上界，不得比 Step 实际需求更宽。Plan 必须通过 DAG、ordinal 唯一性、引用完整性、循环依赖、resource budget 聚合、Case/Assertion/Evidence 聚合和 cleanup coverage 校验。

### 6.4 PolicyDecision

```ts
type ApprovalRequirement =
  | { kind: "required"; reason_codes: string[]; approval_scope_digest: Sha256 }
  | { kind: "not_required"; reason_codes: string[]; authority_rule_id: string };

type DesignPolicyDecision =
  | (ContractMeta & {
      effect: "allow";
      decision_id: string;
      run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
      policy_ref: DigestBoundRef<"qa.policy/v1">;
      design_scope: DesignScope;
      design_scope_digest: Sha256;
      approval_requirement: ApprovalRequirement;
      reason_codes: string[];
      evaluated_at: ISO8601;
    })
  | (ContractMeta & {
      effect: "deny";
      decision_id: string;
      run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
      policy_ref: DigestBoundRef<"qa.policy/v1">;
      approval_requirement: { kind: "not_required"; reason_codes: string[]; authority_rule_id: string };
      reason_codes: [string, ...string[]];
      evaluated_at: ISO8601;
    });

type PolicyViolation = {
  rule_id: string;
  step_id?: string;
  path: string;
  message: string;
};

type PolicyDecision =
  | (ContractMeta & {
      effect: "allow";
      decision_id: string;
      run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
      plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
      policy_ref: DigestBoundRef<"qa.policy/v1">;
      approval_requirement: ApprovalRequirement;
      approved_envelope: ActionEnvelope;
      approved_envelope_digest: Sha256;
      reason_codes: string[];
      violations: PolicyViolation[];
      evaluated_at: ISO8601;
    })
  | (ContractMeta & {
      effect: "deny";
      decision_id: string;
      run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
      plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
      policy_ref: DigestBoundRef<"qa.policy/v1">;
      approval_requirement: { kind: "not_required"; reason_codes: string[]; authority_rule_id: string };
      reason_codes: string[];
      violations: [PolicyViolation, ...PolicyViolation[]];
      evaluated_at: ISO8601;
    });
```

`DesignPolicyDecision` 与 `PolicyDecision` 都是 strict union。Design `effect="allow"` 必须同时携带 `design_scope` 和 digest；deny variant 禁止出现 scope/digest 且必须禁止签发 Design Grant。Design allow 可以 required 或 not_required，但两条路径都必须产生 approved DesignApprovalEvidence。

Execution Policy 的 allow/deny 与“是否要求用户批准”是两个独立结论：只有 `effect="allow"` 才能产生 approved envelope；allow 可以 required 或 not_required，deny 永远禁止签发 Execution Grant。每个 Execution Grant 都必须引用一个 approved ExecutionApprovalEvidence；`not_required` 路径必须由 hosted Authorization Authority 生成 `evidence_basis="policy_not_required"` 的 Evidence，不能省略 Evidence。

Policy evaluator 必须 fail closed。任何无法理解的 command、network destination、Secret、capability、EvidenceRequirement 或 resource 单位必须拒绝。

---

## 7. `PlanAmendmentRequest` 与 `PlanAmendment`

```ts
type AmendmentReasonCode =
  | "new_step"
  | "file_scope_expansion"
  | "network_scope_expansion"
  | "new_secret"
  | "privilege_expansion"
  | "resource_budget_expansion"
  | "assertion_change"
  | "evidence_requirement_change"
  | "other";

type PlanDiffOperation = {
  op: "add" | "remove" | "replace";
  path: string;                    // JSON Pointer
  before_digest?: Sha256;
  after_value?: unknown;
  reason: string;
};

type PlanAmendmentRequest = ContractMeta & {
  request_id: string;
  old_plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  requested_by: { type: "backend" | "runner" | "runtime" | "policy" | "user"; id: string };
  reason_code: AmendmentReasonCode;
  requested_action: PlanAction;
  observed_step_id?: string;
  old_fence: ExecutionFence;
  expected_cursor: RuntimeCursor;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  requested_at: ISO8601;
};

type GrantRevocationReceipt = ContractMeta & {
  receipt_id: string;
  grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
  revoked_sequence: number;
  revoked_at: ISO8601;
  reason: "amendment" | "user" | "timeout" | "grant_revoked" | "policy" | "shutdown" | "security" | "superseded";
  authority: "fkst-hosted.authorization-authority";
  status: "revoked";
  signature: SignatureBlock;
};

type ResumeDirective = {
  resume_id: string;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  earliest_step_id: string;
  reusable_step_ids: string[];
  rerun_step_ids: string[];
  invalidated_effect_ids: string[];
  new_fence: ExecutionFence;
  expected_cursor: RuntimeCursor;
  rationale: string;
};

type PlanAmendment = ContractMeta & {
  amendment_id: string;
  request_ref: DigestBoundRef<"qa.plan-amendment-request/v1">;
  old_plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  old_execution_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
  old_fence: ExecutionFence;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  revocation_receipt_ref: DigestBoundRef<"qa.grant-revocation-receipt/v1">;
  termination_receipt_refs: DigestBoundRef<"qa.termination-receipt/v1">[];
  amendment_cleanup_receipt_ref: DigestBoundRef<"qa.cleanup-receipt/v1">;
  amendment_design_approval_ref: DigestBoundRef<"qa.approval-evidence/v1">;
  amendment_design_local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  amendment_design_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
  new_plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  amendment_design_cleanup_receipt_ref: DigestBoundRef<"qa.cleanup-receipt/v1">;
  operations: PlanDiffOperation[];
  new_policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
  new_approval_evidence_refs: [DigestBoundRef<"qa.approval-evidence/v1">, ...DigestBoundRef<"qa.approval-evidence/v1">[]];
  new_execution_local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  new_execution_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
  resume: ResumeDirective;
  finalized_at: ISO8601;
};
```

`PlanAmendmentRequest` 只是暂停和重新设计的输入，禁止被 Runtime 当作新授权。只有完整 `PlanAmendment` 才可通过 `AmendmentResumeCommand` 恢复执行；它必须同时证明旧 Grant 已撤销、旧 fence 已失效、Checkpoint 已冻结、旧 Backend/进程已终止、旧环境与 CredentialLease 已 Cleanup，amendment Design Approval/LocalLeaseBinding/Grant 已取得，新 Plan 已在全新的 VZ Linux Design VM 中生成且 Design Cleanup 已 settled，新 Plan 已通过 Policy、Execution Evidence 已批准、新 Execution LocalLeaseBinding 与 Grant 已签发且 ResumeDirective 使用新 fence。

触发 Amendment 时必须按以下顺序执行：

1. 以 `PlanAmendmentRequest.old_fence` 停止启动新的 Plan Step，并持久化 cursor。
2. 取消或终止 active Backend，生成 `TerminationReceipt`。
3. 持久化 Checkpoint、Artifact、已完成 effect 和 resource inventory digest。
4. 撤销旧 Execution Grant并生成 `GrantRevocationReceipt`；旧 fence 随后不得再接受 mutating command/event。
5. 使用本地 `CleanupCapability` 执行 `cleanup(reason=amendment_pause)`，销毁旧 Sandbox、进程组、Browser Profile 和 CredentialLease。
6. Policy Gate 检查 amendment design scope；按 required/not_required 规则生成新的 DesignApprovalEvidence；Hosted 先取得未激活 Design LocalLeaseBinding，Hosted Authorization Authority 验证 Evidence/Policy/binding 后签发 amendment Design Grant。
7. Runtime 原子激活 Design reservation，使用新的 SourceObjectLease 创建全新的 VZ Linux Design VM，在原 RunSpec/effective SHA 上生成新 Plan 与结构化 Diff，随后销毁 VM并取得 Design CleanupReceipt。
8. 对新 Plan 重新执行 Policy 和 Execution ApprovalEvidence；Hosted 取得独立 Execution LocalLeaseBinding，Hosted Authorization Authority 验证后签发新 Execution Grant。Reservation 本身不得 fence 旧 generation。
9. 只有上述 Evidence、两个 LocalLeaseBinding、Grant、CleanupReceipt 和 ResumeDirective 全部 digest-bound 后才创建不可变 `PlanAmendment`。
10. `AmendmentResumeCommand(type="resume_amendment")` 必须同时验证 `resume.new_fence`、new execution binding 和新 Execution Grant，并在单写事务中原子激活 reservation 后创建新的 VZ Linux Execution VM；只复用 Checkpoint 明确标记为 reusable 且无外部副作用不确定性的逻辑结果，禁止复用旧 VM/overlay/process/CredentialLease/Chrome profile。Recovery 只能使用独立的 `RecoveryResumeCommand(type="resume_recovery")` 与 signed RecoveryDecision，禁止把 RecoveryDecision 填进 PlanAmendment 或反向复用 Amendment Resume。

新增 Step、扩大文件/网络/Secret/权限、提高资源预算或改变 assertion/EvidenceRequirement 语义必须重新审批。只缩小权限或修正无副作用的显示元数据可以由策略选择 `approval_requirement=not_required`，但仍必须生成新 Plan、PolicyDecision、Execution ApprovalEvidence、Grant 和 Amendment。

---

## 8. Transport-neutral Runtime 协议与 NyxID Adapter

### 8.1 Runtime Service Interface

Runtime 协议必须与 NyxID 私有 API 解耦。逻辑接口定义如下：

```ts
type RuntimeRunState =
  | "accepted"
  | "designing"
  | "design_cleaning_up"
  | "awaiting_execution_grant"
  | "preparing"
  | "ready"
  | "executing"
  | "amendment_required"
  | "collecting_evidence"
  | "cancelling"
  | "timing_out"
  | "cleaning_up"
  | "cleanup_repair"
  | "recovering"
  | "completed"
  | "cancelled"
  | "failed";

type RuntimeRunSnapshot = ContractMeta & {
  runtime_run_id: string;
  runtime_instance_id: string;
  fence: ExecutionFence;
  fence_digest: Sha256;
  cursor: RuntimeCursor;
  hosted_acknowledged_cursor?: RuntimeCursor;
  state: RuntimeRunState;
  accepted_command_ids: string[];
  command_admission_receipt_refs: DigestBoundRef<"qa.command-admission-receipt/v1">[];
  fence_transition_refs: DigestBoundRef<"qa.fence-transition/v1">[];
  predecessor_fencing_record_refs: DigestBoundRef<"qa.predecessor-fencing-record/v1">[];
  stable_environment_ids: string[];
  source_effective_sha: string;
  active_plan_ref?: DigestBoundRef<"qa.structured-plan/v1">;
  active_grant_ref?: DigestBoundRef<"qa.signed-grant/v1">;
  local_lease_binding_ref?: DigestBoundRef<"qa.local-lease-binding/v1">;
  active_local_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
  active_step?: { step_id: string; attempt: number };
  environment_ref?: DigestBoundRef<"qa.prepared-environment/v1">;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_version: number;
  resource_inventory_digest: Sha256;
  inventory_seal_receipt_refs: DigestBoundRef<"qa.inventory-seal-receipt/v1">[];
  effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
  unsettled_effect_states: EffectState[];
  recovery_ledger_snapshot_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  audit_checkpoint_ref: DigestBoundRef<"qa.audit-checkpoint/v1">;
  ledger_integrity_checkpoint_ref: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">;
  ledger_integrity_verification_receipt_ref: DigestBoundRef<"qa.ledger-integrity-verification-receipt/v1">;
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  case_result_refs: DigestBoundRef<"qa.case-result/v1">[];
  evidence_manifest_refs: DigestBoundRef<"qa.evidence-manifest/v1">[];
  termination_receipt_refs: DigestBoundRef<"qa.termination-receipt/v1">[];
  cleanup_receipt_ref?: DigestBoundRef<"qa.cleanup-receipt/v1">;
  last_error?: ErrorEnvelope;
  runtime_signature: SignatureBlock;
};

type RuntimeAdmissionSnapshot = RuntimeScopedMeta & {
  snapshot_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  runtime_boot_epoch: string;
  recovery_epoch: string;
  admission_state: "open" | "closed";
  admission_reason_codes: string[];
  protocol_versions: string[];
  schema_reader_versions: string[];
  schema_writer_versions: string[];
  capability_digest: Sha256;
  compatibility_set_digest: Sha256;
  guest_image_digest: Sha256;
  runtime_hard_ceilings_ref: DigestBoundRef<"qa.runtime-hard-ceilings/v1">;
  runtime_hard_ceilings_digest: Sha256;
  capacity: {
    active_run_slots_total: number;
    active_run_slots_available: number;
    reservation_slots_available: number;
    vm_slots_available: number;
    browser_slots_available: number;
    port_slots_available: number;
    available_cpu_millis: number;
    available_memory_bytes: number;
    available_disk_bytes: number;
    available_process_count: number;
    available_open_file_count: number;
    host_storage_headroom_bytes: number;
  };
  pressure: {
    disk: "normal" | "warning" | "critical";
    ledger: "normal" | "warning" | "critical";
    outbox: "normal" | "warning" | "critical";
  };
  browser_enforcement_capability_ref?: DigestBoundRef<"qa.browser-enforcement-capability/v1">;
  secret_broker_health_ref?: DigestBoundRef<"qa.secret-broker-health/v1">;
  captured_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type AdmissionRequirements = {
  required_runtime_instance_id: string;
  required_device_id: string;
  required_protocol_version: string;
  required_schema_majors: string[];
  required_capabilities: string[];
  required_compatibility_set_digest: Sha256;
  required_guest_image_digest: Sha256;
  resource_class: string;
  active_run_slots: number;
  reservation_slots: number;
  vm_slots: number;
  port_count: number;
  browser_slots: number;
  secret_lease_count: number;
  cpu_millis: number;
  memory_bytes: number;
  disk_bytes: number;
  process_count: number;
  open_file_count: number;
  wall_clock_seconds: number;
  host_storage_bytes: number;
  minimum_emergency_headroom_bytes: number;
  requested_ttl_seconds: number;
  admission_deadline_at: ISO8601;
};

type AdmissionRequirementsDigest = Sha256; // `admission_requirements/v1` projection

type DesignAdmissionAuthorizationPreimage = {
  kind: "design";
  phase: "design" | "amendment_design";
  run_id: UUID;
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source_object_ref: DigestBoundRef<"qa.source-object/v1">;
  source_effective_sha: string;
  design_policy_decision_ref: DigestBoundRef<"qa.design-policy-decision/v1">;
  design_scope_digest: Sha256;
  project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  device_id: string;
  runtime_instance_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  hosted_workflow: LeaseFence;
  expected_predecessor?: LeaseFence;
  admission_requirements: AdmissionRequirements;
};

type ExecutionAdmissionAuthorizationPreimage = {
  kind: "execution";
  phase: "execution" | "amendment_execution";
  run_id: UUID;
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  source_effective_sha: string;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
  approved_envelope_digest: Sha256;
  project_profile_ref: DigestBoundRef<"qa.project-profile/v1">;
  device_id: string;
  runtime_instance_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  hosted_workflow: LeaseFence;
  expected_predecessor?: LeaseFence;
  admission_requirements: AdmissionRequirements;
};

type AdmissionAuthorizationPreimage = DesignAdmissionAuthorizationPreimage | ExecutionAdmissionAuthorizationPreimage;

type ReserveLocalLeaseRequestBase = ContractMeta & {
  reservation_id: string;
  authorization_input_digest: Sha256;
  idempotency_key: string;
  request_digest: Sha256;
};

type ReserveLocalLeaseRequest =
  | (ReserveLocalLeaseRequestBase & { kind: "design"; authorization_preimage: DesignAdmissionAuthorizationPreimage })
  | (ReserveLocalLeaseRequestBase & { kind: "execution"; authorization_preimage: ExecutionAdmissionAuthorizationPreimage });

type ReserveLocalLeaseResponse = {
  binding: LocalLeaseBinding;
  binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  disposition: "new" | "idempotent_replay";
  accepted_authorization_input_digest: Sha256;
  active_local_execution_unchanged: true;
};

type AdmissionPredecessor =
  | { kind: "initial"; predecessor_absent: true }
  | {
      kind: "takeover";
      predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
      predecessor_lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">;
      predecessor_fence: ExecutionFence;
      predecessor_last_cursor: RuntimeCursor;
    };

type CommandAdmissionReceiptBase = ContractMeta & {
  receipt_id: string;
  command_ref: DigestBoundRef<"qa.runtime-command/v1">;
  command_id: string;
  command_request_digest: Sha256;
  idempotency_key: string;
  active_local_lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">;
  accepted_cursor: RuntimeCursor;
  outbox_high_watermark: number;
  accepted_at: ISO8601;
  runtime_signature: SignatureBlock;
};

type CommandAdmissionReceipt =
  | (CommandAdmissionReceiptBase & {
      admission_kind: "grant_lease_activation";
      local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
      authorization_input_digest: Sha256;
      admission_requirements_digest: AdmissionRequirementsDigest;
      predecessor: AdmissionPredecessor;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      stable_environment_id: string;
      initial_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      initial_inventory_version: 1;
      initial_inventory_digest: Sha256;
      cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      first_event_cursor: RuntimeCursor;
    })
  | (CommandAdmissionReceiptBase & {
      admission_kind: "recovery_lease_activation";
      recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">;
      new_local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
      new_execution_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      authorization_input_digest: Sha256;
      admission_requirements_digest: AdmissionRequirementsDigest;
      predecessor: AdmissionPredecessor;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      stable_environment_id: string;
      reconciled_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      reconciled_inventory_version: number;
      reconciled_inventory_digest: Sha256;
      cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      first_event_cursor: RuntimeCursor;
    })
  | (CommandAdmissionReceiptBase & {
      admission_kind: "control_quiesce_reconcile_activation";
      authority_ref: DigestBoundRef<"qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1">;
      predecessor: Extract<AdmissionPredecessor, { kind: "takeover" }>;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      stable_environment_id: string;
      open_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      open_inventory_version: number;
      first_event_cursor: RuntimeCursor;
    })
  | (CommandAdmissionReceiptBase & {
      admission_kind: "control_cleanup_activation";
      authority_ref: DigestBoundRef<"qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1" | "qa.repair-operation/v1">;
      predecessor: Extract<AdmissionPredecessor, { kind: "takeover" }>;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      stable_environment_id: string;
      sealed_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      sealed_inventory_version: number;
      sealed_inventory_digest: Sha256;
      inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
      successor_cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      first_event_cursor: RuntimeCursor;
    })
  | (CommandAdmissionReceiptBase & {
      admission_kind: "existing_lease";
      lease_purpose: "execution" | "control_quiesce_reconcile" | "control_cleanup";
      authority_ref: DigestBoundRef<"qa.signed-grant/v1" | "qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1" | "qa.repair-operation/v1">;
    });

type LocalRequestAuthentication = RuntimeScopedMeta & {
  authentication_id: string;
  local_ipc_binding_ref: DigestBoundRef<"qa.local-ipc-binding/v1">;
  session_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  runtime_boot_epoch: string;
  session_epoch: number;
  caller_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  direction: "client_to_runtime";
  method: "probeHealth" | "reserveLocalLeaseBinding" | "submitCommand" | "getRun" | "streamEvents" | "ackEvents" | "getArtifact" | "cancelReservation";
  request_digest: Sha256;
  request_sequence: number;
  previous_request_digest?: Sha256;
  nonce: string;
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type AuthenticatedLocalRequest = { authentication: LocalRequestAuthentication };

type LocalResponseAuthentication = RuntimeScopedMeta & {
  authentication_id: string;
  local_ipc_binding_ref: DigestBoundRef<"qa.local-ipc-binding/v1">;
  session_id: string;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  runtime_boot_epoch: string;
  session_epoch: number;
  direction: "runtime_to_client";
  request_digest: Sha256;
  response_digest: Sha256;
  response_sequence: number;
  previous_response_digest?: Sha256;
  nonce: string;
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type LocalTransportResponse<T> = { authentication: LocalResponseAuthentication; payload: T };

type ProbeHealthRequest =
  | { detail: "public"; correlation_id?: string }
  | ({ detail: "authenticated"; correlation_id?: string } & AuthenticatedLocalRequest);
type ProbeHealthResponse = { health: RuntimeHealth };
type SubmitCommandRequest = AuthenticatedLocalRequest & { command: RuntimeCommand; request_digest: Sha256 };
type SubmitCommandResponse = { disposition: "new" | "idempotent_replay"; admission_receipt: CommandAdmissionReceipt; snapshot: RuntimeRunSnapshot };
type GetRunRequest = AuthenticatedLocalRequest & { run_id: UUID; expected_generation?: number };
type GetRunResponse = { snapshot: RuntimeRunSnapshot };

type RuntimeStreamPosition =
  | { kind: "from_first"; generation: number }
  | { kind: "after"; cursor: RuntimeCursor };

type StreamEventsRequest = AuthenticatedLocalRequest & { run_id: UUID; position: RuntimeStreamPosition; max_batch_events?: number };
type AckEventsRequest = AuthenticatedLocalRequest & { run_id: UUID; through_cursor: RuntimeCursor; event_set_digest: Sha256; idempotency_key: string };
type AckEventsResponse = { acknowledged_through: RuntimeCursor; durable: true };

type ArtifactAccessCapability = ContractMeta & {
  capability_id: string;
  issuer: "fkst-hosted.artifact-access-authority";
  subject: ActorRef;
  audience: {
    runtime_instance_id: string;
    artifact_store_provider?: string;
  };
  artifact_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
  allowed_operations: ["read"];
  allowed_ranges: Array<{ offset: number; length: number }> | "full";
  revocation_id: string;
  revocation_feed_ref: DigestBoundRef<"qa.revocation-feed/v1">;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RevocationFact =
  | {
      kind: "grant";
      fact_id: string;
      fact_sequence: number;
      grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      grant_sequence: number;
      effective_at: ISO8601;
      reason: "cancelled" | "timed_out" | "amendment" | "security" | "superseded";
      required_action: "quiesce_non_cleanup_effects";
    }
  | {
      kind: "artifact_access";
      fact_id: string;
      fact_sequence: number;
      access_capability_ref: DigestBoundRef<"qa.artifact-access-capability/v1">;
      revocation_id: string;
      artifact_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
      subject: ActorRef;
      effective_at: ISO8601;
      reason: "grant_revoked" | "run_revoked" | "user_access_removed" | "legal_hold_changed" | "security" | "capability_superseded";
      required_action: "deny_future_reads";
    };

type RevocationBatch = RuntimeScopedMeta & {
  batch_id: string;
  issuer: "fkst-hosted.revocation-authority";
  audience: "fkst-local-qa-runtime-transport-control";
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  sequence: number;
  previous_batch_ref?: DigestBoundRef<"qa.revocation-batch/v1">;
  previous_batch_digest?: Sha256;
  facts: [RevocationFact, ...RevocationFact[]];
  watermark: {
    grant_fact_sequence: number;
    artifact_access_fact_sequence: number;
  };
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RevocationDeliveryReceiptBase = RuntimeScopedMeta & {
  receipt_id: string;
  batch_ref: DigestBoundRef<"qa.revocation-batch/v1">;
  batch_sequence: number;
  accepted_previous_batch_digest?: Sha256;
  accepted_batch_digest: Sha256;
  applied_watermark: RevocationBatch["watermark"];
  acknowledged_at: ISO8601;
  signature: SignatureBlock;
};

type RevocationDeliveryReceipt =
  | (RevocationDeliveryReceiptBase & { disposition: "applied" })
  | (RevocationDeliveryReceiptBase & {
      disposition: "idempotent_replay";
      original_receipt_ref: DigestBoundRef<"qa.revocation-delivery-receipt/v1">;
    });

type RuntimeTransportControlInbox = {
  deliverRevocations(batch: RevocationBatch): Promise<RevocationDeliveryReceipt>;
};

type GetArtifactRequest = AuthenticatedLocalRequest & {
  run_id: UUID;
  actor: ActorRef;
  artifact_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
  access_capability: ArtifactAccessCapability;
  range?: { offset: number; length: number };
};
type GetArtifactResponse = { artifact: ArtifactPointer; bytes_digest: Sha256; content: AsyncIterable<Uint8Array> };

type CancelReservationRequest = AuthenticatedLocalRequest & {
  reservation_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
  request_digest: Sha256;
  idempotency_key: string;
  reason: "user_cancelled" | "authorization_abandoned" | "superseded" | "drain";
};
type CancelReservationResponse = { disposition: "cancelled" | "already_cancelled" | "already_expired"; active_local_execution_unchanged: true; cancelled_at: ISO8601 };

type RuntimeService = {
  probeHealth(request: ProbeHealthRequest): Promise<ProbeHealthResponse>;
  reserveLocalLeaseBinding(request: ReserveLocalLeaseRequest & AuthenticatedLocalRequest): Promise<ReserveLocalLeaseResponse>;
  cancelReservation(request: CancelReservationRequest): Promise<CancelReservationResponse>;
  submitCommand(request: SubmitCommandRequest): Promise<SubmitCommandResponse>;
  getRun(request: GetRunRequest): Promise<GetRunResponse>;
  streamEvents(request: StreamEventsRequest): AsyncIterable<RuntimeEventBatch>;
  ackEvents(request: AckEventsRequest): Promise<AckEventsResponse>;
  getArtifact(request: GetArtifactRequest): Promise<GetArtifactResponse>;
};
```

`RuntimeService` 必须保持且只能保持以上八个业务方法；`LocalTransportResponse` 是 wire authentication envelope，不新增业务方法。每个 active `LocalIPCBinding` 必须唯一绑定 Runtime identity/pairing/boot/session epoch 和一组 client executable identities。请求链与响应链分别从 sequence=1 开始并独立持久化 high watermark、最后 digest 和已用 nonce；sequence=1 禁止携带 previous digest，sequence>1 必须携带前一已接受同方向消息的 digest。首次接收必须严格等于 durable high watermark+1；gap、倒退、不同 payload 复用 sequence、nonce 或 authentication id 均 fail closed。完全相同的 `(binding_ref, direction, sequence, digest, nonce)` transport replay 只能返回原持久化结果，不得再次进入业务幂等或副作用路径。

Runtime restart、identity rotation、re-pair、pairing revocation、client binary replacement、协议 rekey 或 binding expiry 必须原子把旧 binding 标为 `retired` 并递增相应 epoch；retired binding 禁止接受任何新 sequence 或 nonce。Crash 后 sequence/nonce ledger 必须先恢复再开放 authenticated traffic，禁止因内存状态丢失把旧请求当新请求。`LocalRequestAuthentication` 的 identity/pairing/session fields 必须与解析后的 active binding 和当前 Runtime 状态逐项相等，且 `request_digest` 必须使用 `local_request/v1` 重算；request/response transport 签名分别使用 §3.6 `purpose="local_ipc_request"` 与 `purpose="local_ipc_response"`。

`RuntimeTransportControlInbox` 是独立于八方法 `RuntimeService` 的 transport control ingress，只允许接收完整、签名、exact-object 的 `RevocationBatch`，禁止承载 RuntimeCommand、Grant、Plan、配置、任意 JSON payload 或通用消息。Batch 与 durable ack 必须分别按 §3.6 `purpose="revocation_batch"` 和 `purpose="revocation_delivery_receipt"` 验签。Batch 必须按目标 `(runtime_instance_id, identity_epoch, pairing_epoch)` 建立从 sequence=1 开始的单调 hash chain；首批禁止 previous 字段，后续批次的 ref/digest 必须精确指向最近 durable accepted batch。Runtime 必须先按 `(batch_id, content_digest)` 查找幂等结果，再检查 freshness、nonce、sequence、previous digest 和 watermark；同 id 同 digest 返回原 `RevocationDeliveryReceipt`，同 id 不同 digest、gap、rollback、过期 batch 或 watermark 倒退必须拒绝且不得推进任何 revocation state。

Grant 与 Artifact access 两类 fact 使用独立单调 `fact_sequence` 和同一 batch ack。Batch 一旦 durable apply，Runtime 必须在同一 transaction 中更新相应 revocation watermark、阻断后续 Grant effect 或 Artifact read、写审计 outbox，并持久化签名 Receipt；ack 只确认该 batch 及 watermark 已 durable apply，不表示取消/Cleanup 已完成。连接中断、Node 重放或 Runtime restart 后，完全相同 batch 必须取得 idempotent ack；freshness 超过 Grant/Capability 声明上限、发现 chain gap 或 pairing 被撤销时，Runtime 必须关闭相应 admission/read path并请求 snapshot/re-delivery，禁止把“暂未收到撤销”解释为继续授权。

Grant 签发前必须执行 `reserveLocalLeaseBinding`：hosted 持有有效 HostedWorkflowLease，经认证调用目标 Runtime；Runtime 必须先按 `(idempotency_key, request_digest)` 查询 reservation 幂等记录，再做任何可变 reservation、fence、cursor、capacity 或 nonce 检查。同 key、同 digest 返回原 binding；同 key、不同 digest 拒绝且不得改动任何状态。首次请求通过后，Runtime 对 strict Design/Execution variant 做 exact-object 校验，以 `authorization_preimage` 的完整 JCS bytes 重算 `authorization_input_digest`，再在 SQLite 中幂等写入 inert reservation并返回签名 `LocalLeaseBinding`。Design variant 禁止 Plan、Execution Policy 与 approved envelope 字段；Execution variant 禁止 source object、Design Policy 与 Design scope 字段。Authorization Authority 必须独立重建同一 exact preimage 并验证 digest，禁止把 binding 跨 phase、Plan、Policy、device、runtime 或 requirements 复用。

`RuntimeAdmissionSnapshot` 必须由 Runtime device-bound key 签名，并在短 TTL 内绑定 Runtime identity/pairing refs/epochs、boot/recovery epoch、schema reader/writer、capability/image、RuntimeHardCeilings、Browser/Secret capability、容量和 disk/ledger/outbox pressure。`reserveLocalLeaseBinding` 必须从该 snapshot 原子创建 `quota_hold_digest`，只扣减 `AdmissionRequirements` 声明的逻辑 slot；snapshot 过期、quota 不足、Browser capability 不存在或 emergency headroom 不足时 fail closed。Hosted 与 Runtime 都必须用 `admission_requirements/v1` 重算完整 digest，禁止只比较 required capabilities 字符串。

`reserveLocalLeaseBinding` 禁止改变 active lease、cursor 或 predecessor 状态。只有 `submitCommand` 的 Grant admission transaction 可以激活 reservation。同一 binding 只能激活一次；成功和幂等重放都必须返回持久化的 `CommandAdmissionReceipt`。`RuntimeStreamPosition(kind="from_first")` 从指定 generation 的第一个持久化事件开始，第一项 cursor 必须严格等于 `{ generation, sequence: 1 }`；`kind="after"` 只返回更大 sequence。`RuntimeCursor.sequence=0` 只可作为尚无事件时的本地 before-first sentinel，禁止出现在 RuntimeEvent、ack 或持久化 Receipt 的 `first_event_cursor`。`ackEvents` 只推进 hosted delivery acknowledgement，不得删除未进入 retention window 的 Event，也不得推进 Run 状态。

Cancel 必须通过 §8.3 的 `CancelCommand` 提交，禁止存在绕过 `CommandPrecondition`、`CommandTarget`、command sequence 和 signed intent 的独立 `cancelRun` Interface。`RuntimeRunState` 是设备侧执行快照，不是云端 `WorkflowState`；hosted 只能根据 RuntimeEvent、Snapshot、Checkpoint 和 Receipt 推进 workflow，不得按枚举名称直接等同。

Runtime v1 可以通过 loopback HTTP + authenticated streaming 实现，但 wire transport 禁止改变上述语义。

### 8.2 NyxID Adapter 映射

| Runtime 操作 | NyxID 下行/上行映射 | 要求 |
|---|---|---|
| `probeHealth` | Cloud/Node authenticated query → Runtime | public/authenticated detail 分级；不得触发 command 或 reservation。 |
| `reserveLocalLeaseBinding` | Cloud request → Node route → loopback Runtime | Node 只透传 strict request、request digest 和签名 binding response；reservation 禁止 fence active generation。 |
| `cancelReservation` | Cloud request → Node route → loopback Runtime | 只取消 inert reservation并释放逻辑 quota hold；幂等记录阻止晚到 Grant activation，不改变 active lease/fence/cursor。 |
| `submitCommand` | Cloud request → Node route → loopback Runtime | Node 不解包或修改 signed Grant/Capability/RecoveryDecision；必须透传 request digest、correlation id、fence 和 cursor。 |
| `getRun` | Cloud query → Node route → Runtime snapshot | 返回值必须保留所有 digest、fence、transition、seal 和 cursor。 |
| `streamEvents` / `ackEvents` | Runtime event → Node → Cloud ingestion/ack | 允许断线重连；按 `(run_id, generation, sequence)` 去重并显式确认 durable cursor。 |
| `getArtifact` | Authenticated Cloud fetch → Node → Runtime | 只可读取 post-redaction ArtifactPointer；raw quarantine 永不可下载。 |
| `RuntimeTransportControlInbox.deliverRevocations` | signed revocation batch → Node transport control → Runtime | 独立于八方法 RuntimeService；只传 Grant/Artifact access revocation facts、hash chain 与 idempotent ack，禁止通用 command/config payload。 |
| `CancelCommand` / `CleanupCommand` | fenced `submitCommand` → Node → Runtime | 取消与清理必须走与其他 mutating command 相同的鉴权、幂等和顺序检查。 |

NyxID Adapter 必须：

- 使用 Node 主动建立的出站安全连接，禁止要求用户开放公网入站端口。
- 将 NyxID Approval/Device Attestation 映射为 `ApprovalEvidence`。
- 传输由 hosted Authorization Authority 签发的 Grant，禁止重签、扩权或改写 claims。
- 对 loopback Runtime 使用生产级本地认证；禁止保留 POC 的 `auth_method=none`。
- 把 Node routing error 与 Runtime application error 分开编码。

NyxID Adapter 禁止执行 Plan Step、注入 Secret、推导 Pass/Fail、缓存 Authorization Authority 私钥，或在转发时替换 fence/cursor。

### 8.3 RuntimeCommand

```ts
type CommandPrecondition = {
  observed_fence?: ExecutionFence;       // initial admission 时不存在
  expected_predecessor_cursor?: RuntimeCursor;
  expected_predecessor_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
  observed_runtime_snapshot_ref?: DigestBoundRef<"qa.runtime-run-snapshot/v1">;
};

type CommandTarget = {
  target_fence: ExecutionFence;
  target_initial_cursor: RuntimeCursor;  // target generation + sequence=0
  transition_purpose: "execution" | "control_quiesce_reconcile" | "control_cleanup";
};

type FencedCommandBase = ContractMeta & {
  command_id: string;
  idempotency_key: string;
  command_sequence: number;              // 对 target generation 从 1 开始单调递增
  precondition: CommandPrecondition;
  target: CommandTarget;
  deadline_at: ISO8601;
};

type DesignCommand = FencedCommandBase & {
  type: "design";
  run_spec: RunSpec;
  source_object_lease: SourceObjectLease;
  design_grant: SignedGrant<DesignGrantClaims>;
};

type ExecuteCommand = FencedCommandBase & {
  type: "execute";
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  plan: StructuredPlan;
  execution_grant: SignedGrant<ExecutionGrantClaims>;
};

type AmendmentResumeCommand = FencedCommandBase & {
  type: "resume_amendment";
  amendment: PlanAmendment;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  execution_grant: SignedGrant<ExecutionGrantClaims>;
};

type RecoveryResumeCommand = FencedCommandBase & {
  type: "resume_recovery";
  recovery_decision: Extract<RecoveryDecision, { decision: "resume" }>;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  runtime_snapshot_ref: DigestBoundRef<"qa.runtime-run-snapshot/v1">;
  recovery_ledger_snapshot_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  new_local_lease_binding: Extract<LocalLeaseBinding, { phase: "execution" | "amendment_execution" }>;
  new_execution_grant: SignedGrant<ExecutionGrantClaims>;
};

type ResumeCommand = AmendmentResumeCommand | RecoveryResumeCommand;

type CancellationReason = "user" | "timeout" | "grant_revoked" | "policy" | "shutdown" | "amendment";

type FenceTransitionAuthorization = ContractMeta & {
  authorization_id: string;
  issuer: "fkst-hosted.authorization-authority" | "fkst-local-qa-runtime.timeout-authority";
  purpose: "execution" | "control_quiesce_reconcile" | "control_cleanup";
  observed_fence?: ExecutionFence;
  target_fence: ExecutionFence;
  expected_predecessor_cursor?: RuntimeCursor;
  authority_ref: DigestBoundRef<
    | "qa.signed-grant/v1"
    | "qa.cancellation-intent/v1"
    | "qa.timeout-intent/v1"
    | "qa.recovery-decision/v1"
    | "qa.repair-operation/v1"
  >;
  allowed_operations: Array<"step" | "quiesce" | "reconcile" | "seal" | "terminate" | "revoke" | "release" | "delete">;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type CancellationIntent = ContractMeta & {
  cancellation_intent_id: string;
  issuer: "fkst-hosted.authorization-authority";
  reason: CancellationReason;
  requested_by: ActorRef;
  observed_fence: ExecutionFence;
  expected_predecessor_cursor: RuntimeCursor;
  target_quiesce_fence: ExecutionFence;
  grant_revocation_receipt_ref: DigestBoundRef<"qa.grant-revocation-receipt/v1">;
  quiesce_transition_authorization_ref: DigestBoundRef<"qa.fence-transition-authorization/v1">;
  requested_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type TimeoutIntentBase = ContractMeta & {
  timeout_intent_id: string;
  issuer: "fkst-hosted.workflow-authority" | "fkst-local-qa-runtime.timeout-authority";
  observed_fence: ExecutionFence;
  expected_cursor: RuntimeCursor;
  absolute_deadline_at: ISO8601;
  detected_at: ISO8601;
  runtime_boot_epoch?: string;
  monotonic_elapsed_millis?: number;
  nonce: string;
  signature: SignatureBlock;
};

type TimeoutIntent =
  | (TimeoutIntentBase & { kind: "command"; command_ref: DigestBoundRef<"qa.runtime-command/v1"> })
  | (TimeoutIntentBase & { kind: "phase"; phase: "design" | "execution" | "cleanup" | "recovery" })
  | (TimeoutIntentBase & { kind: "step"; step_id: string; attempt: number })
  | (TimeoutIntentBase & { kind: "warden"; process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1"> })
  | (TimeoutIntentBase & { kind: "lease_expiry"; local_lease_ref: DigestBoundRef<"qa.local-execution-lease/v1"> });

本地 `fkst-local-qa-runtime.timeout-authority` 只能为当前 runtime instance/boot epoch 已到期的 command/phase/step/warden/lease签发 `TimeoutIntent` 和 `FenceTransitionAuthorization(purpose="control_quiesce_reconcile")`，target generation 只能是 observed local generation 的下一个合法 successor，allowed operations 只能是 quiesce/reconcile/terminate/revoke。它不能签 execution purpose、扩大 Hosted fence、创建 Grant、改变 Plan 或直接授权 release/delete。inventory seal 后，Runtime cleanup authority可基于原 TimeoutIntent与 SealReceipt签发等权或更窄 CleanupCapability successor，并激活 `control_cleanup`。

type TerminationControlIntent =
  | { kind: "cancellation"; cancellation_intent: CancellationIntent }
  | { kind: "timeout"; timeout_intent: TimeoutIntent };

type CancelCommand = FencedCommandBase & {
  type: "cancel";
  termination_intent: TerminationControlIntent;
  transition_authorization: FenceTransitionAuthorization;
};

type OwnerCleanupAuthority = {
  kind: "active_owner";
  active_local_lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">;
};

type HigherFenceCleanupAuthority =
  | {
      kind: "cancellation_takeover";
      cancellation_intent_ref: DigestBoundRef<"qa.cancellation-intent/v1">;
      grant_revocation_receipt_ref: DigestBoundRef<"qa.grant-revocation-receipt/v1">;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
    }
  | {
      kind: "timeout_takeover";
      timeout_intent_ref: DigestBoundRef<"qa.timeout-intent/v1">;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
    }
  | {
      kind: "recovery_takeover";
      recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
    }
  | {
      kind: "repair_takeover";
      repair_operation_ref: DigestBoundRef<"qa.repair-operation/v1">;
      fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
      predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
    };

type CleanupCommandBase = FencedCommandBase & {
  cleanup_capability: CleanupCapability;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_version: number;
  resource_inventory_digest: Sha256;
  inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
  cleanup_lineage_id: string;
  cleanup_attempt: number;
  reason: CleanupReason;
};

type CleanupCommand =
  | (CleanupCommandBase & { type: "cleanup"; authority: OwnerCleanupAuthority })
  | (CleanupCommandBase & { type: "cleanup_takeover"; authority: HigherFenceCleanupAuthority });

type RuntimeCommand =
  | DesignCommand
  | ExecuteCommand
  | ResumeCommand
  | CancelCommand
  | CleanupCommand;
```

所有 RuntimeCommand 都是 fenced mutating command；健康探测只允许走 `RuntimeService.probeHealth`。Design/Execute/Amendment Resume 的 `target.target_fence` 必须等于所携 Grant 的 `authorized_fence`，并与 Grant 的 `local_lease_binding_ref` 完整匹配；Recovery Resume 必须等于签名 RecoveryDecision 的 target fence。`precondition` 只描述已观察 predecessor 事实，`target` 只描述 commit 后的 generation/fence/cursor，禁止把两者压成同一个 fence/cursor。每次 `submitCommand` 必须在解析或消费 Grant nonce、读取可变 reservation、比较 fence/cursor/sequence、seal inventory 或执行任何副作用前，先查询 `(idempotency_key, request_digest)`：同 key、同 digest 返回原 `CommandAdmissionReceipt`，不同 digest 返回冲突且保持全部状态不变。

首次 admission 才进入 single-writer transaction，并同时验证 hosted/local fence、deadline、command sequence、CommandPrecondition/Target 和 command-specific authority。Cancel 首先激活更高的 `control_quiesce_reconcile` fence，只可 suppress/quiesce/reconcile/terminate/revoke；seal transaction 成功后，独立的 `cleanup_takeover` 才能使用更高 `control_cleanup` fence、sealed inventory 和 successor capability执行 release/delete。owner cleanup 必须绑定当前 active lease。所有 Cleanup 都必须绑定同一 lineage 的最新 sealed snapshot ref/version/digest 与 seal receipt；裸 digest、open snapshot、旧 version 或跨 lineage snapshot必须在副作用前拒绝。

### 8.4 RuntimeEvent

```ts
type RuntimeEventCause =
  | {
      kind: "command";
      command_id: string;
      command_sequence: number;
      idempotency_key: string;
    }
  | {
      kind: "effect";
      effect_id: string;
      effect_record_ref: DigestBoundRef<"qa.effect-record/v1">;
      command_id: string;
      command_sequence: number;
      idempotency_key: string;
    }
  | {
      kind: "recovery";
      recovery_attempt_id: string;
      recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">;
      recovery_ledger_snapshot_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
      command_id: string;
      command_sequence: number;
      idempotency_key: string;
    }
  | {
      kind: "runtime";
      reason: "heartbeat" | "startup_reconciliation" | "shutdown" | "update";
    };

type RuntimeEventBase = ContractMeta & {
  event_id: string;
  runtime_instance_id: string;
  runtime_run_id: string;
  generation: number;
  cursor: RuntimeCursor;
  fence_digest: Sha256;
  cause: RuntimeEventCause;
  occurred_at: ISO8601;
};

type RuntimeEvent = RuntimeEventBase & (
  | { type: "command_accepted"; command_type: RuntimeCommand["type"]; admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">; accepted_cursor: RuntimeCursor }
  | { type: "local_lease_activated_from_binding"; lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">; binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">; predecessor: AdmissionPredecessor }
  | { type: "local_lease_activated_from_recovery"; lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">; recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">; predecessor: Extract<AdmissionPredecessor, { kind: "takeover" }> }
  | { type: "control_quiesce_reconcile_lease_activated"; lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">; authority_ref: DigestBoundRef<"qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">; predecessor: Extract<AdmissionPredecessor, { kind: "takeover" }> }
  | { type: "control_cleanup_lease_activated"; lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">; authority_ref: DigestBoundRef<"qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1" | "qa.repair-operation/v1">; successor_cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">; predecessor: Extract<AdmissionPredecessor, { kind: "takeover" }> }
  | { type: "cleanup_capability_issued"; capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">; environment_id: string; inventory_lineage_root_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1"> }
  | { type: "state_changed"; from: RuntimeRunState; to: RuntimeRunState; reason_code: string }
  | { type: "heartbeat"; state: RuntimeRunState; active_step_id?: string; inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">; inventory_digest: Sha256; admission_open: boolean }
  | { type: "effect_state_changed"; effect_record_ref: DigestBoundRef<"qa.effect-record/v1">; from: EffectState; to: EffectState; effect_receipt_ref?: DigestBoundRef<"qa.effect-receipt/v1"> }
  | { type: "effect_settled"; effect_record_ref: DigestBoundRef<"qa.effect-record/v1">; effect_receipt_ref: DigestBoundRef<"qa.effect-receipt/v1">; terminal_state: Extract<EffectState, "settled"> }
  | { type: "inventory_updated"; inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">; previous_version: number; new_version: number }
  | { type: "inventory_sealed"; inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">; inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">; sealed_version: number; sealed_digest: Sha256 }
  | { type: "step_started"; plan_ref: DigestBoundRef<"qa.structured-plan/v1">; step_id: string; attempt: number }
  | { type: "step_observation"; observation_ref: DigestBoundRef<"qa.backend-observation/v1">; step_id: string; attempt: number }
  | { type: "step_completed"; step_id: string; attempt: number; case_result_refs: DigestBoundRef<"qa.case-result/v1">[] }
  | {
      type: "plan_generated";
      plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
      source_effective_sha: string;
      design_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      design_environment_receipt_ref: DigestBoundRef<"qa.design-environment-receipt/v1">;
    }
  | { type: "amendment_required"; request_ref: DigestBoundRef<"qa.plan-amendment-request/v1"> }
  | { type: "observation_sanitized"; observation_ref: DigestBoundRef<"qa.sanitized-observation/v1">; redaction_receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1"> }
  | { type: "artifact_registered"; artifact_ref: DigestBoundRef<"qa.artifact-pointer/v1">; redaction_receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1">; requirement_ids: string[] }
  | { type: "cleanup_started"; cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">; inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">; inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">; inventory_digest: Sha256 }
  | { type: "cleanup_completed"; cleanup_receipt_ref: DigestBoundRef<"qa.cleanup-receipt/v1"> }
  | { type: "repair_completed"; repair_receipt_ref: DigestBoundRef<"qa.runtime-repair-receipt/v1"> }
  | { type: "termination_completed"; termination_receipt_ref: DigestBoundRef<"qa.termination-receipt/v1"> }
  | { type: "error"; error: ErrorEnvelope }
  | { type: "run_snapshot"; snapshot_ref: DigestBoundRef<"qa.runtime-run-snapshot/v1"> }
);

type RuntimeEventBatch = RuntimeScopedMeta & {
  batch_id: string;
  run_id: UUID;
  generation: number;
  from_cursor: RuntimeCursor;
  through_cursor: RuntimeCursor;
  events: [RuntimeEvent, ...RuntimeEvent[]];
  event_set_digest: Sha256;
  referenced_runtime_fact_refs: DigestBoundRef[];
  emitted_at: ISO8601;
  signature: SignatureBlock;
};
```

`RuntimeEventCause` 与 `RuntimeEvent` 均是 strict union。command/effect/recovery cause 的 `generation` 和 `cursor.generation` 必须等于对应 active `ExecutionFence.local_execution.generation`，`fence_digest` 必须等于完整 ExecutionFence 的 JCS digest；effect/recovery cause 的 `command_id`、`command_sequence` 和 `idempotency_key` 必须逐项等于触发该事件的 durable admitted RuntimeCommand，effect cause 还必须绑定持久化 EffectRecord。只有 heartbeat、启动对账、shutdown 和 update 可以使用 runtime cause；禁止以虚构 command id 填充非命令事件。`local_lease_activated_from_binding.predecessor` 必须与同一 admission receipt 的 `AdmissionPredecessor` 完全相等：initial variant 只允许 `{ kind="initial", predecessor_absent=true }`，禁止要求或携带 `PredecessorFencingRecord`；takeover variant 必须携带真实 predecessor record/lease/fence/cursor。Recovery 和两个 control activation event 只能使用 takeover predecessor。`plan_generated` 缺少任一绑定字段时禁止进入 Policy Review。

每个被激活的 local generation 必须以 `command_accepted` 的 cursor `{generation, sequence: 1}` 开始，此前不得存在该 generation 的 Event；同 generation 后续 sequence 必须无间隙严格递增。`streamEvents` 必须返回由 Runtime device-bound key 签名的 `RuntimeEventBatch`，并用 `event_set/v1` 绑定批内 cursor/digest 以及引用的 Receipt/Snapshot；单独的未签名 Event 不构成来源认证。Cloud ingestion 可以接受完全相同的重复事件，但必须拒绝旧 generation、相同 cursor 不同 digest、sequence 倒退和已失效 fence 事件；出现 gap 时必须请求 snapshot 或使用 `RuntimeStreamPosition(kind="after")` 重连，禁止猜测缺失事件。只有 `ackEvents` 成功持久化的 cursor 才是 hosted delivery checkpoint；接收 Event 本身不等于 acknowledgement。

---

## 9. `EnvironmentFactory`、Capability、Credential 与 Sandbox 生命周期

### 9.1 Resource Inventory、CleanupCapability 与 CredentialLease

```ts
type ExecutableIdentity = ContractMeta & {
  executable_identity_id: string;
  execution_domain: "host" | "vz_guest";
  immutable_object_ref: DigestBoundRef<"qa.executable-object/v1">;
  byte_digest: Sha256;
  code_signing?: { team_id: string; signing_identifier: string; designated_requirement_digest: Sha256; notarized: boolean };
  guest_image_digest?: Sha256;
  role: "rust_runtime" | "typescript_worker" | "guest_agent" | "app" | "middleware" | "backend" | "browser" | "controlled_helper";
};

type ProcessLaunchBinding = ContractMeta & {
  launch_binding_id: string;
  environment_id: string;
  effect_id: string;
  launch_intent_digest: Sha256;      // executable/argv/cwd/descriptors/secret refs 的独立 projection，不含本对象 digest
  executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  argv_digest: Sha256;
  working_directory: RootQualifiedPath;
  inherited_descriptor_policy_digest: Sha256;
  authorized_secret_refs: string[];
  authorized_secret_injection_modes: Array<"environment" | "file" | "proxy">;
  fence_digest: Sha256;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
  launch_nonce: string;
};

type LocalIPCBindingBase = RuntimeScopedMeta & {
  binding_id: string;
  session_id: string;
  transport: { kind: "unix_domain_socket"; endpoint_token: string } | { kind: "loopback_mtls"; port: number; certificate_digest: Sha256 };
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  runtime_boot_epoch: string;
  session_epoch: number;
  server_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  allowed_client_executable_identity_refs: DigestBoundRef<"qa.executable-identity/v1">[];
  peer_credential_policy_digest: Sha256;
  protocol_versions: string[];
  audience: "fkst-local-qa-runtime-control";
  direction_policy: "independent_strict_hash_chains";
  client_to_runtime_initial_sequence: 1;
  runtime_to_client_initial_sequence: 1;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type LocalIPCBinding =
  | (LocalIPCBindingBase & { status: "active" })
  | (LocalIPCBindingBase & {
      status: "retired";
      retired_at: ISO8601;
      retire_reason: "runtime_restart" | "identity_rotation" | "re_pair" | "pairing_revoked" | "client_binary_replaced" | "protocol_rekey" | "expired";
      successor_binding_ref?: DigestBoundRef<"qa.local-ipc-binding/v1">;
    });

type GuestBootEvidence = ContractMeta & {
  evidence_id: string;
  runtime_instance_id: string;
  runtime_boot_epoch: string;
  vm_instance_id: string;
  guest_boot_id: string;
  sandbox_descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
  boot_chain: {
    bootloader_digest: Sha256;
    kernel_digest: Sha256;
    initrd_digest: Sha256;
    rootfs_digest: Sha256;
    kernel_command_line_digest: Sha256;
    guest_agent_digest: Sha256;
  };
  host_challenge_nonce: string;
  guest_ephemeral_public_key: Base64UrlNoPad;
  evidence_kind: "host_verified_boot_manifest_and_guest_challenge_response";
  verified_at: ISO8601;
  signature: SignatureBlock;
};

type BootBoundAuthenticatedVsockSession = ContractMeta & {
  session_id: string;
  environment_id: string;
  phase: "design" | "amendment_design" | "execution" | "amendment_execution";
  generation: number;
  fence_digest: Sha256;
  runtime_instance_id: string;
  runtime_boot_epoch: string;
  vm_instance_id: string;
  guest_boot_id: string;
  sandbox_descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
  guest_boot_evidence_ref: DigestBoundRef<"qa.guest-boot-evidence/v1">;
  guest_agent_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  host_challenge_nonce: string;
  host_ephemeral_public_key: Base64UrlNoPad;
  guest_ephemeral_public_key: Base64UrlNoPad;
  key_agreement: "x25519";
  key_derivation: "hkdf-sha256";
  channel_protection: "chacha20-poly1305";
  protocol_version: string;
  channel_binding_digest: Sha256;
  host_to_guest_initial_sequence: 1;
  guest_to_host_initial_sequence: 1;
  host_to_guest_high_watermark: number;
  guest_to_host_high_watermark: number;
  replay_window: 0;
  bootstrap_material_erased: true;
  established_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type VsockMessageEnvelope = ContractMeta & {
  message_id: string;
  session_ref: DigestBoundRef<"qa.boot-bound-authenticated-vsock-session/v1">;
  direction: "host_to_guest" | "guest_to_host";
  sequence: number;
  previous_message_digest?: Sha256;
  payload_schema_version: string;
  payload_digest: Sha256;
  sent_at: ISO8601;
  channel_auth_tag: Base64UrlNoPad;
};

`GuestBootEvidence` 证明 Supervisor 验证了自己启动的 VZ boot manifest，且 guest 持有本次 VM 唯一 bootstrap material并完成 challenge-response；它不是硬件 remote attestation，也不证明 compromised guest kernel 之后的运行时完整性。bootstrap material、ephemeral key 和 session key 禁止跨 VM、boot、Runtime boot epoch、generation 或 snapshot 复用。HKDF salt/info 必须绑定 `channel_transcript/v1` digest；host-to-guest 与 guest-to-host 使用不同 key。任一 sequence 回退、gap、重复、boot/runtime restart 或 transcript mismatch 都必须终止 session并进入 VM Cleanup。

type ProcessIdentity = ContractMeta & {
  process_identity_id: string;
  runtime_instance_id: string;
  environment_id: string;
  execution_domain: "host" | "vz_guest";
  role: ExecutableIdentity["role"];
  pid: number;
  process_start_token: string;      // boot/session/start-time 组合的不透明稳定标识
  parent_process_identity_id?: string;
  process_group_identity: string;
  process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
  executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  launch_binding_ref: DigestBoundRef<"qa.process-launch-binding/v1">;
};

type ResourceUsageReceipt = ContractMeta & {
  receipt_id: string;
  environment_id: string;
  resource_limit_binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  process_identity_refs: DigestBoundRef[];
  interval: { started_at: ISO8601; ended_at: ISO8601 };
  cpu_time_millis: number;
  peak_memory_bytes: number;
  disk_read_bytes: number;
  disk_written_bytes: number;
  network_ingress_bytes: number;
  network_egress_bytes: number;
  peak_process_count: number;
  peak_open_file_count: number;
  source: "host_warden" | "guest_agent" | "vz_provider";
};

type ProcessDomainDescriptor = ContractMeta & {
  process_domain_id: string;
  environment_id: string;
  role: "guest_agent" | "typescript_worker" | "app" | "middleware" | "backend" | "browser" | "secret_broker" | "controlled_helper";
  execution_domain: "host" | "vz_guest";
  uid: number;
  gid: number;
  supplemental_gids: number[];
  namespace_policy: {
    user: "private" | "host_managed";
    pid: "private" | "host_managed";
    mount: "private" | "host_managed";
    network: "deny_all" | "mediated_gateway" | "host_enforced_proxy";
  };
  resource_limit_binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  enforcement_config_digest: Sha256;
  linux_capabilities: string[];
  no_new_privileges: true;
  seccomp_policy_digest?: Sha256;
  allowed_ipc_audiences: string[];
  child_policy: "no_children" | "same_domain_only" | "declared_child_domains";
  allowed_child_domain_refs: DigestBoundRef<"qa.process-domain-descriptor/v1">[];
  secret_exposure: "none" | "proxy_only" | "guest_injector" | "target_and_declared_descendants";
  core_dump: "disabled";
  ptrace: "denied_outside_domain";
  termination_root: boolean;
  fence_digest: Sha256;
};

type ProcessWardenScope = ContractMeta & {
  warden_scope_id: string;
  environment_id: string;
  host_warden_id: string;
  guest_warden_id?: string;
  process_group_identity: string;
  process_domain_refs: [DigestBoundRef<"qa.process-domain-descriptor/v1">, ...DigestBoundRef<"qa.process-domain-descriptor/v1">[]];
  local_ipc_binding_ref: DigestBoundRef<"qa.local-ipc-binding/v1">;
  boot_bound_authenticated_vsock_session_ref?: DigestBoundRef<"qa.boot-bound-authenticated-vsock-session/v1">;
  fence_digest: Sha256;
  created_at: ISO8601;
};

type InventoryResource = {
  resource: DigestBoundRef;
  owner_run_id: UUID;
  owner_environment_id: string;
  category:
    | "process"
    | "process_group"
    | "port"
    | "endpoint"
    | "file"
    | "directory"
    | "mount"
    | "sandbox"
    | "vm_instance"
    | "vsock_session"
    | "local_ipc_session"
    | "browser_session"
    | "browser_profile"
    | "network_proxy"
    | "credential_lease"
    | "secret_materialization"
    | "artifact_staging"
    | "raw_quarantine"
    | "update_staging";
  process_identity?: ProcessIdentity;
  ownership_tag: string;
  cleanup_action: "terminate" | "release" | "delete" | "revoke";
  lifecycle: "active" | "settled" | "preserved";
  preserve_policy_ref?: DigestBoundRef<"qa.retention-policy/v1">;
};

type ResourceInventorySnapshot = ContractMeta & {
  inventory_id: string;             // 同一 lineage 所有 version 稳定不变
  lineage_id: string;
  environment_id: string;
  version: number;                  // 从 1 开始严格递增
  previous_snapshot_ref?: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  state: "open" | "sealed";
  resources: InventoryResource[];
  inventory_digest: Sha256;         // resources 的 JCS digest
  created_by_effect_id?: string;
  created_at: ISO8601;
  seal_barrier_sequence?: number;
  sealed_at?: ISO8601;
};

type InventorySealReceipt =
  | (ContractMeta & {
      kind: "sealed";
      receipt_id: string;
      inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      inventory_id: string;
      lineage_id: string;
      environment_id: string;
      sealed_version: number;
      sealed_digest: Sha256;
      barrier_sequence: number;
      barrier_effect_set_digest: Sha256;
      unsettled_pre_barrier_effect_ids: [];
      sealed_event_cursor: RuntimeCursor;
      sealed_at: ISO8601;
      signature: SignatureBlock;
    })
  | (ContractMeta & {
      kind: "rejected";
      receipt_id: string;
      inventory_id: string;
      lineage_id: string;
      requested_version: number;
      barrier_sequence: number;
      unsettled_pre_barrier_effect_ids: [string, ...string[]];
      error: ErrorEnvelope;
      rejected_at: ISO8601;
      signature: SignatureBlock;
    });

type CleanupCapabilityBase = ContractMeta & {
  capability_id: string;
  issuer: "fkst-local-qa-runtime.cleanup-authority";
  runtime_instance_id: string;
  issued_fence_digest: Sha256;
  owner_local_generation: number;
  environment_ids: [string, ...string[]];
  resource_inventory_id: string;
  inventory_lineage_id: string;
  inventory_lineage_root_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  inventory_lineage_root_digest: Sha256;
  allowed_actions: Array<"terminate" | "release" | "delete" | "revoke">;
  allowed_reasons: CleanupReason[];
  issued_at: ISO8601;
  not_before: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type CleanupCapability =
  | (CleanupCapabilityBase & { kind: "initial"; capability_sequence: 1 })
  | (CleanupCapabilityBase & {
      kind: "successor";
      capability_sequence: number;
      predecessor_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      successor_authority:
        | { kind: "cancellation"; cancellation_intent_ref: DigestBoundRef<"qa.cancellation-intent/v1">; inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "timeout"; timeout_intent_ref: DigestBoundRef<"qa.timeout-intent/v1">; inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "recovery"; recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "repair"; repair_operation_ref: DigestBoundRef<"qa.repair-operation/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> };
    });

type CredentialLease = ContractMeta & {
  lease_id: string;
  issuer: "fkst-local-secret-broker";
  runtime_instance_id: string;
  environment_id: string;
  broker_binding_ref: DigestBoundRef<"qa.secret-broker-binding/v1">;
  broker_boot_epoch: string;
  secret_ref: string;                 // opaque reference，不是 Secret 值
  sealed_lease_handle_ref: string;    // broker-only sealed handle；Ledger 只保存其 digest/ref
  authorized_process_domain_refs: [DigestBoundRef<"qa.process-domain-descriptor/v1">, ...DigestBoundRef<"qa.process-domain-descriptor/v1">[]];
  authorized_process_launch_binding_refs: [DigestBoundRef<"qa.process-launch-binding/v1">, ...DigestBoundRef<"qa.process-launch-binding/v1">[]];
  authorized_executable_identity_refs: [DigestBoundRef<"qa.executable-identity/v1">, ...DigestBoundRef<"qa.executable-identity/v1">[]];
  step_ids: string[];
  inject_as: "environment" | "file" | "proxy";
  allowed_destinations: string[];
  fence_digest: Sha256;
  issued_at: ISO8601;
  expires_at: ISO8601;
  renewable: boolean;
};

type CredentialLeaseReceipt = ContractMeta & {
  receipt_id: string;
  lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
  action: "issued" | "renewed" | "revoked" | "expired" | "reconciled";
  status: "active" | "settled" | "failed";
  observed_at: ISO8601;
  error?: ErrorEnvelope;
  signature: SignatureBlock;
};

type SecretMaterializationReceipt =
  | (ContractMeta & {
      kind: "materialized";
      receipt_id: string;
      lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
      effect_receipt_ref: DigestBoundRef<"qa.effect-receipt/v1">;
      target_process: ProcessIdentity;
      process_launch_binding_ref: DigestBoundRef<"qa.process-launch-binding/v1">;
      executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
      injection: "environment" | "file" | "proxy";
      opaque_materialization_ref: string;
      materialized_at: ISO8601;
      expires_at: ISO8601;
      signature: SignatureBlock;
    })
  | (ContractMeta & {
      kind: "released";
      receipt_id: string;
      lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
      materialization_receipt_ref: DigestBoundRef<"qa.secret-materialization-receipt/v1">;
      released_at: ISO8601;
      release_reason: CleanupReason;
      signature: SignatureBlock;
    })
  | (ContractMeta & {
      kind: "failed";
      receipt_id: string;
      lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
      target_process?: ProcessIdentity;
      process_launch_binding_ref?: DigestBoundRef<"qa.process-launch-binding/v1">;
      executable_identity_ref?: DigestBoundRef<"qa.executable-identity/v1">;
      failed_at: ISO8601;
      error: ErrorEnvelope;
      signature: SignatureBlock;
    });
```

`ResourceInventorySnapshot` 必须是 append-only version chain。每个创建、更新或结算资源的 Effect 必须在同一 SQLite transaction 中校验 previous version 并写入新 snapshot；同一 environment 同一时刻只能有一个 current version。请求 seal 时 Runtime 必须先持久化 barrier sequence，停止接受该 barrier 之前的新增资源 effect，并等待所有 `cursor/effect admission <= barrier` 的 EffectRecord 达到可判定状态；只有 `unsettled_pre_barrier_effect_ids=[]` 才能创建新的 `state="sealed"` snapshot 与签名 `InventorySealReceipt(kind="sealed")`。Seal receipt、sealed snapshot 与 `inventory_sealed` outbox event 必须原子提交。Cleanup 只能接受该 receipt 精确绑定的 ref/version/digest；裸 digest、open snapshot、旧 seal receipt、跨 lineage snapshot或“最后写覆盖历史”均禁止。`inventory_digest` 相同不代表 snapshot version 可互换。

`CleanupCapability` 必须在第一个本地资源副作用前、与稳定 environment id 和空 inventory lineage root 同一 admission transaction 创建并持久化，且不能依赖 Execution Grant 仍然有效；它只能清理 capability 中列出的 environment、同一 inventory id/lineage id/root 的已验证 descendant snapshot、action 和 reason。Cleanup 开始时必须另行绑定最新 sealed snapshot/ref/version/digest 和 `InventorySealReceipt`；禁止让 capability 预先猜测未来 inventory digest，也禁止用不同 lineage 的 snapshot。签名字节按 §3.6，`purpose="cleanup_capability"`；InventorySealReceipt 使用 `purpose="inventory_seal_receipt"`。

Capability 的 `issued_fence_digest` 记录创建资源时的原 fence，而不是要求所有后续 cleanup command 复用旧 fence。正常 Cleanup 使用同 generation 的 active-owner authority。Runtime restart、owner takeover 或 terminal repair 必须先以 signed RecoveryDecision/RepairOperation 和 `FenceTransition(purpose="control_cleanup")` 创建严格收窄的新 CleanupCapability：`capability_sequence` 必须递增，`predecessor_capability_ref` 必须指向旧 capability，environment/lineage/actions/reasons/expiry 只能相同或更窄，successor nonce 必须新鲜；旧 capability 随 successor activation 被记为 superseded。更高 fence 只能携带 successor capability 和 `cleanup_takeover` command，禁止借旧 capability 创建资源、执行 Step、恢复测试或取得新 CredentialLease。`CredentialLeaseReceipt` 使用 §3.6 `purpose="credential_receipt"`。

Process Warden 必须以 `(execution_domain, pid, process_start_token, process_group_identity, environment_id)` 识别进程。禁止仅按 PID、进程名、端口或可执行文件路径终止进程；PID reuse、guest reboot 或 host reboot 后必须先 reconcile identity。宿主 Rust Warden 负责 VZ VM、guest agent、host Chrome 与受控 host helper；VZ guest agent/Warden 负责 VM 内 TypeScript worker、App、Middleware 和 Backend。两者的 Receipt 必须汇入同一 inventory version chain。

Secret 值、`sealed_lease_handle_ref` 和实际 materialization location 禁止离开独立 Secret Broker helper 与获准 process domain；Supervisor、Ledger 和 Process Warden 只持有 digest-bound reference、ownership metadata 和签名 Receipt。`opaque_materialization_ref` 只能由当前 broker boot epoch 解析。Secret materialization 必须同时匹配 CredentialLease、ProcessLaunchBinding、ExecutableIdentity、实际 ProcessIdentity、Step、destination 与 fence；PID、进程名或 role 相同不能替代 executable/launch binding。每次 materialize/release/fail 都必须产生 strict `SecretMaterializationReceipt`，签名字节按 §3.6 且 `purpose="secret_materialization_receipt"`。hosted、NyxID、TypeScript worker、普通 Event 和 Artifact Store 只能看到 digest-bound lease/receipt reference。

### 9.2 EnvironmentFactory Interface

```ts
type RuntimeHardCeilings = RuntimeScopedMeta & {
  ceilings_id: string;
  issuer: "fkst-runtime-release-authority";
  runtime_identity_epoch: number;
  applies_to_release_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
  per_sandbox: {
    cpu_millis: number;
    memory_bytes: number;
    writable_disk_bytes: number;
    process_count: number;
    open_file_count: number;
    wall_clock_seconds: number;
    network_egress_bytes: number;
    dependency_download_bytes: number;
    artifact_quarantine_bytes: number;
  };
  per_runtime: {
    active_sandboxes: number;
    active_browser_sessions: number;
    active_port_count: number;
    aggregate_memory_bytes: number;
    aggregate_writable_disk_bytes: number;
    minimum_emergency_headroom_bytes: number;
  };
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type DependencyAcquisitionPolicy = ContractMeta & {
  policy_id: string;
  phase: "design" | "execution";
  allowed_package_managers: Array<"npm" | "pnpm" | "yarn" | "cargo" | "pip" | "bundler" | "maven" | "gradle">;
  lockfiles: [RootQualifiedPath, ...RootQualifiedPath[]];
  require_frozen_lockfile: true;
  require_declared_integrity: true;
  allowed_integrity_algorithms: Array<"sha256" | "sha384" | "sha512">;
  registries: Array<{
    scheme: "https";
    host: string;
    port: 443;
    path_prefixes: string[];
    registry_identity_ref: DigestBoundRef<"qa.dependency-registry-identity/v1">;
  }>;
  lifecycle_scripts: "deny" | "allow_declared_digest_bound";
  allowed_script_digests: Sha256[];
  git_dependencies: "deny" | "exact_commit_and_archive_digest";
  local_path_dependencies: "sandbox_roots_only";
  cache: "verified_read_only" | "disabled";
  approved_network_envelope_digest: Sha256;
  maximum_dependency_count: number;
  maximum_download_bytes: number;
  maximum_elapsed_seconds: number;
  fail_on_unpinned_transitive_dependency: true;
};

type DependencyAcquisitionReceipt =
  | (ContractMeta & {
      kind: "acquired";
      receipt_id: string;
      policy_ref: DigestBoundRef<"qa.dependency-acquisition-policy/v1">;
      environment_id: string;
      lockfile_refs: [DigestBoundRef<"qa.sandbox-file/v1">, ...DigestBoundRef<"qa.sandbox-file/v1">[]];
      package_manager: DependencyAcquisitionPolicy["allowed_package_managers"][number];
      package_manager_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
      dependencies: Array<{
        name: string;
        version: string;
        source_url_digest: Sha256;
        registry_identity_ref: DigestBoundRef<"qa.dependency-registry-identity/v1">;
        expected_integrity: string;
        verified_archive_digest: Sha256;
        signature_or_provenance_ref?: DigestBoundRef;
        source: "verified_cache" | "network";
      }>;
      lifecycle_script_launch_binding_refs: DigestBoundRef<"qa.process-launch-binding/v1">[];
      network_flow_receipt_refs: DigestBoundRef<"qa.network-flow-receipt/v1">[];
      total_download_bytes: number;
      completed_at: ISO8601;
      signature: SignatureBlock;
    })
  | (ContractMeta & {
      kind: "rejected";
      receipt_id: string;
      policy_ref: DigestBoundRef<"qa.dependency-acquisition-policy/v1">;
      environment_id: string;
      package_manager?: DependencyAcquisitionPolicy["allowed_package_managers"][number];
      failed_dependency?: { name: string; version?: string; source_url_digest?: Sha256 };
      network_flow_receipt_refs: DigestBoundRef<"qa.network-flow-receipt/v1">[];
      reason: "missing_lockfile" | "lockfile_changed" | "unpinned_dependency" | "integrity_missing" | "integrity_mismatch" | "registry_denied" | "script_denied" | "budget_exceeded" | "timeout";
      error: ErrorEnvelope;
      rejected_at: ISO8601;
      signature: SignatureBlock;
    });

type ResourceLimitBinding = ContractMeta & {
  binding_id: string;
  environment_id: string;
  runtime_hard_ceilings_ref: DigestBoundRef<"qa.runtime-hard-ceilings/v1">;
  admission_requirements_digest: AdmissionRequirementsDigest;
  approved_envelope_digest: Sha256;
  effective_limits: {
    cpu_millis: number;
    memory_bytes: number;
    writable_disk_bytes: number;
    process_count: number;
    open_file_count: number;
    wall_clock_seconds: number;
    network_egress_bytes: number;
    dependency_download_bytes: number;
    artifact_quarantine_bytes: number;
  };
  enforcement: {
    host_warden: true;
    vz_configuration: true;
    guest_cgroup_v2: true;
    guest_rlimit: true;
    mediated_network_meter: true;
    storage_quota: true;
  };
  enforcement_config_digest: Sha256;
  fence_digest: Sha256;
  issued_at: ISO8601;
  signature: SignatureBlock;
};

type ResourceLimitReceiptBase = ContractMeta & {
  receipt_id: string;
  binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  environment_id: string;
  signature: SignatureBlock;
};

type ResourceLimitReceipt =
  | (ResourceLimitReceiptBase & { kind: "applied"; enforcement_config_digest: Sha256; applied_at: ISO8601 })
  | (ResourceLimitReceiptBase & {
      kind: "violated";
      limit: keyof ResourceLimitBinding["effective_limits"];
      limit_value: number;
      observed_value: number;
      observed_usage_ref: DigestBoundRef<"qa.resource-usage-receipt/v1">;
      enforcement_action: "deny" | "throttle" | "terminate_process_domain" | "stop_vm";
      termination_receipt_ref?: DigestBoundRef<"qa.termination-receipt/v1">;
      violated_at: ISO8601;
    })
  | (ResourceLimitReceiptBase & { kind: "released"; final_usage_ref: DigestBoundRef<"qa.resource-usage-receipt/v1">; released_at: ISO8601 });

type NetworkFlowReceiptBase = ContractMeta & {
  receipt_id: string;
  environment_id: string;
  effect_request_ref: DigestBoundRef<"qa.effect-request/v1">;
  process_identity_ref: DigestBoundRef<"qa.process-identity/v1">;
  policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1" | "qa.design-policy-decision/v1">;
  grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
  resource_limit_binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  requested: { scheme: "https" | "http" | "tcp"; host: string; port: number; purpose: string };
  resolved_ip_addresses: string[];
  dns_answer_digest: Sha256;
  proxy_binding_ref: DigestBoundRef<"qa.network-proxy-binding/v1">;
  checked_at: ISO8601;
  signature: SignatureBlock;
};

type NetworkFlowReceipt =
  | (NetworkFlowReceiptBase & {
      kind: "allowed";
      matched_destination_digest: Sha256;
      connection_id: string;
      ingress_bytes: number;
      egress_bytes: number;
      opened_at: ISO8601;
      closed_at?: ISO8601;
    })
  | (NetworkFlowReceiptBase & {
      kind: "denied";
      reason: "destination_not_approved" | "private_or_metadata_address" | "dns_rebinding" | "direct_socket" | "protocol_not_allowed" | "egress_budget_exceeded" | "enforcer_unavailable";
      blocked_before_connect: true;
      denied_at: ISO8601;
    });

type VZSandboxDescriptor = ContractMeta & {
  descriptor_id: string;
  environment_id: string;
  purpose: "design" | "execution";
  provider: "apple_virtualization_framework_linux_vm";
  vm_image_ref: DigestBoundRef<"qa.vz-linux-image/v1">;
  vm_image_digest: Sha256;
  boot_chain: {
    bootloader_digest: Sha256;
    kernel_digest: Sha256;
    initrd_digest: Sha256;
    rootfs_digest: Sha256;
    kernel_command_line_digest: Sha256;
  };
  guest_agent: {
    protocol_version: string;
    binary_digest: Sha256;
    bootstrap_nonce: string;
  };
  dependency_acquisition_policy_ref: DigestBoundRef<"qa.dependency-acquisition-policy/v1">;
  runtime_hard_ceilings_ref: DigestBoundRef<"qa.runtime-hard-ceilings/v1">;
  resource_limit_binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  redaction_policy_ref: DigestBoundRef<"qa.redaction-policy/v1">;
  resources: {
    cpu_count: number;
    memory_bytes: number;
    disk_bytes: number;
  };
  roots: Array<{ name: SandboxRootName; root_identity: string; host_source_token: string; guest_path: string; mode: "read_only" | "read_write" }>;
  mounts: Array<{
    root: SandboxRootName;
    source_token: string;
    guest_path: string;
    mode: "read_only" | "read_write";
    purpose: "source" | "workspace" | "artifact_staging" | "runtime_metadata";
  }>;
  vsock: {
    protocol_version: string;
    host_challenge_nonce: string;
    expected_guest_agent_digest: Sha256;
    key_agreement: "x25519";
    key_derivation: "hkdf-sha256";
    channel_protection: "chacha20-poly1305";
  };
  network: {
    mode: "deny_all" | "mediated_allowlist";
    bridge_id?: string;
    destinations: ActionEnvelope["network"]["destinations"];
    record_every_flow: true;
    deny_private_link_local_metadata_and_host_loopback: true;
  };
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
  fence_digest: Sha256;
};

type VZSandboxReceipt =
  | (ContractMeta & {
      kind: "launched";
      receipt_id: string;
      descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
      vm_instance_id: string;
      guest_boot_id: string;
      guest_boot_evidence_digest: Sha256;
      launched_at: ISO8601;
      inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
    })
  | (ContractMeta & {
      kind: "partial_failure";
      receipt_id: string;
      descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
      vm_instance_id?: string;
      failed_stage: "allocate" | "configure" | "boot" | "guest_boot_authentication";
      inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      error: ErrorEnvelope;
      failed_at: ISO8601;
    })
  | (ContractMeta & {
      kind: "failed_without_resources";
      receipt_id: string;
      descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
      error: ErrorEnvelope;
      failed_at: ISO8601;
    });

type DesignEnvironmentReceipt = ContractMeta & {
  receipt_id: string;
  design_environment_id: string;
  stable_environment_id: string;
  sandbox_roots: Array<{ name: SandboxRootName; root_identity: string }>;
  source_effective_sha: string;
  source_object_digest: Sha256;
  sandbox_descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
  sandbox_receipt_ref: DigestBoundRef<"qa.vz-sandbox-receipt/v1">;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
  boot_bound_authenticated_vsock_session_ref: DigestBoundRef<"qa.boot-bound-authenticated-vsock-session/v1">;
  dependency_acquisition_receipt_ref: DigestBoundRef<"qa.dependency-acquisition-receipt/v1">;
  resource_limit_binding_ref: DigestBoundRef<"qa.resource-limit-binding/v1">;
  resource_limit_receipt_refs: DigestBoundRef<"qa.resource-limit-receipt/v1">[];
  network_flow_receipt_refs: DigestBoundRef<"qa.network-flow-receipt/v1">[];
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_digest: Sha256;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
};

type PreparedEnvironment = ContractMeta & {
  environment_id: string;
  stable_environment_id: string;
  sandbox_roots: Array<{ name: SandboxRootName; root_identity: string }>;
  source_effective_sha: string;
  source_object_digest: Sha256;
  sandbox_descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1">;
  sandbox_receipt_ref: DigestBoundRef<"qa.vz-sandbox-receipt/v1">;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
  boot_bound_authenticated_vsock_session_ref: DigestBoundRef<"qa.boot-bound-authenticated-vsock-session/v1">;
  allocated_ports: number[];
  endpoint_refs: Array<{ name: string; url: string }>;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_digest: Sha256;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
};

type PrepareDesignResult =
  | (ContractMeta & { kind: "prepared"; result_id: string; environment: DesignEnvironmentReceipt })
  | (ContractMeta & {
      kind: "partial_failure";
      result_id: string;
      design_environment_id: string;
      sandbox_receipt_ref?: DigestBoundRef<"qa.vz-sandbox-receipt/v1">;
      process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
      resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      error: ErrorEnvelope;
    })
  | (ContractMeta & { kind: "failed_without_resources"; result_id: string; error: ErrorEnvelope });

type PrepareExecutionResult =
  | (ContractMeta & { kind: "prepared"; result_id: string; environment: PreparedEnvironment })
  | (ContractMeta & {
      kind: "partial_failure";
      result_id: string;
      environment_id: string;
      sandbox_receipt_ref?: DigestBoundRef<"qa.vz-sandbox-receipt/v1">;
      process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
      resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      error: ErrorEnvelope;
    })
  | (ContractMeta & { kind: "failed_without_resources"; result_id: string; error: ErrorEnvelope });

type ReadinessCheck =
  | { type: "process"; process_ref: string }
  | { type: "tcp"; host: string; port: number }
  | { type: "http"; url: string; expected_status: number[]; body_match?: string }
  | { type: "browser"; url: string; selector?: string };

type EnvironmentFactory = {
  prepareDesign(input: {
    run_spec: RunSpec;
    design_grant: SignedGrant<DesignGrantClaims>;
    admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">;
    bootstrap_context: DesignBootstrapEffectContext;
    fence: ExecutionFence;
    idempotency_key: string;
  }): Promise<PrepareDesignResult>;

  prepareExecution(input: {
    run_spec: RunSpec;
    plan: StructuredPlan;
    execution_grant: SignedGrant<ExecutionGrantClaims>;
    admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">;
    bootstrap_context: ExecutionBootstrapEffectContext;
    fence: ExecutionFence;
    idempotency_key: string;
  }): Promise<PrepareExecutionResult>;

  checkReadiness(input: {
    run_id: UUID;
    environment_ref: DigestBoundRef<"qa.prepared-environment/v1">;
    checks: ReadinessCheck[];
    attempt: number;
    fence: ExecutionFence;
  }): Promise<ReadinessReceipt>;

  cleanup(input: {
    run_id: UUID;
    cleanup_capability: CleanupCapability;
    resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
    resource_inventory_version: number;
    resource_inventory_digest: Sha256;
    inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
    cleanup_lineage_id: string;
    cleanup_attempt: number;
    reason: CleanupReason;
    idempotency_key: string;
  }): Promise<CleanupReceipt>;
};
```

Design 阶段必须先调用 `prepareDesign`，在独立、受限的 VZ Linux Design Sandbox 中生成 `StructuredPlan`。Runtime 必须通过 `plan_generated` 事件返回 Plan、Source、Design Grant 和 DesignEnvironmentReceipt 的 digest-bound reference；hosted 校验后仍必须进入 `design_cleaning_up`。只有 Local Design Sandbox 的 Cleanup settled 后才能进入 `policy_review`，禁止等待或复用 Execution Cleanup。

`VZSandboxDescriptor` 必须在 VM 副作用前冻结。Rust Runtime 必须验证 bootloader/kernel/initrd/rootfs/kernel command line、VM image 和 guest agent digest，使用 `Virtualization.framework` 创建 VM，并要求 bootstrap nonce、GuestBootEvidence 与 descriptor 一致。Source 必须只读挂载；Workspace 和 Artifact staging 必须是 Run 独占 share；网络必须 deny-all 或经过 host EffectGate 的 mediated allowlist。Guest agent 禁止直接访问 host filesystem、SQLite ledger、Secret Broker 或 Chrome automation channel。

Runtime 必须在任何 untrusted binary、包管理器、项目配置、lifecycle script、App 或 Browser 启动前解析并验签 `RuntimeHardCeilings`，以 admission requirements、Plan/Policy/Grant envelope 与 hard ceilings 的逐字段最小值创建 `ResourceLimitBinding`，并在 VZ、host Warden、guest cgroup/rlimit、network meter 和 storage quota 全部 apply 后才允许执行。调用方、Plan、Grant 或管理员配置均不得放宽 hard ceiling；任一 enforcement adapter 缺失、binding digest 不一致或 apply receipt 不完整都必须拒绝整个 phase。达到 wall clock、memory、disk、process、open-file、network、dependency 或 quarantine ceiling 时必须产生 `ResourceLimitReceipt(kind="violated")` 并执行声明的 deny/throttle/termination，禁止只记录告警后继续。

依赖获取只能作为 `EffectRequest(kind="dependency_acquire")` 执行，并严格消费 frozen lockfile、`DependencyAcquisitionPolicy`、批准 registry identity、integrity/provenance 和 budget。Design 默认 `lifecycle_scripts="deny"`；任何允许脚本都必须有声明的 digest、独立 ProcessLaunchBinding、同一 Sandbox/limit/network enforcement 和 Receipt。缺 lockfile、浮动 transitive dependency、registry redirect 越界、integrity 不符、下载预算超限或响应无法与 lockfile 对账时必须 fail closed，禁止 fallback 到未验证 cache、公共 registry、git branch 或重新生成 lockfile。每次 DNS/redirect/connect（含依赖、App、Browser 和受控 helper）都必须经 mediated gateway 生成 strict `NetworkFlowReceipt`；禁止只在流程结束汇总字节数，禁止 direct socket、DNS rebinding、私网/link-local/metadata/host-loopback 绕过。`RuntimeHardCeilings`、`ResourceLimitBinding`、`DependencyAcquisitionReceipt`、`ResourceLimitReceipt` 和 `NetworkFlowReceipt` 必须分别按 §3.6 的同名 purpose 由受信 release authority 或当前 device-bound Runtime key 签名；缺签名的配置/Receipt 不能满足 M2 Gate。

`PrepareDesignResult`、`PrepareExecutionResult` 和 `VZSandboxReceipt` 都必须按 strict union 校验。只要已创建 VM、process、port、directory、mount、CredentialLease 或其他本地资源，prepare 失败就必须返回 `partial_failure`，携带最新 inventory snapshot 与 CleanupCapability，并立即进入 Cleanup；禁止返回裸 error 丢失部分资源。只有能够权威证明没有创建任何资源时才可返回 `failed_without_resources`。

### 9.3 ReadinessReceipt

```ts
type ReadinessReceipt = ContractMeta & {
  receipt_id: string;
  environment_ref: DigestBoundRef<"qa.prepared-environment/v1">;
  attempt: number;
  checks: Array<{
    check: ReadinessCheck;
    outcome: "ready" | "not_ready" | "error";
    observed_at: ISO8601;
    latency_ms?: number;
    error?: ErrorEnvelope;
  }>;
  overall: "ready" | "not_ready" | "failed";
  endpoint_refs: Array<{ name: string; url: string }>;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_version: number;
  resource_inventory_digest: Sha256;
};
```

### 9.4 Local PEP 与 SecretBroker Client

```ts
type EffectRequestBase = ContractMeta & {
  effect_id: string;
  idempotency_key: string;
  request_digest: Sha256;
  step_id?: string;
  attempt?: number;
};

type EffectRequest = EffectRequestBase & (
  | { kind: "file"; operation: "read" | "write" | "delete"; path: RootQualifiedPath; content_ref?: DigestBoundRef<"qa.action-content/v1"> }
  | {
      kind: "process_spawn";
      execution_domain: "host" | "vz_guest";
      executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
      process_launch_binding: ProcessLaunchBinding;
      argv: Array<{ kind: "literal"; value: string } | { kind: "sandbox_path"; value: RootQualifiedPath }>;
      working_directory: RootQualifiedPath;
      secret_lease_refs: DigestBoundRef<"qa.credential-lease/v1">[];
    }
  | { kind: "process_signal"; target_process: ProcessIdentity; signal: "term" | "kill" | "interrupt"; reason: TerminationReason }
  | { kind: "network"; operation: "connect"; scheme: "https" | "http" | "tcp"; host: string; port: number; purpose: string; proxy_binding_ref?: DigestBoundRef<"qa.network-proxy-binding/v1"> }
  | {
      kind: "dependency_acquire";
      policy_ref: DigestBoundRef<"qa.dependency-acquisition-policy/v1">;
      lockfile_refs: [DigestBoundRef<"qa.sandbox-file/v1">, ...DigestBoundRef<"qa.sandbox-file/v1">[]];
      package_manager: DependencyAcquisitionPolicy["allowed_package_managers"][number];
    }
  | { kind: "browser"; action: BrowserAction }
  | {
      kind: "secret_materialize";
      lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
      process_launch_binding_ref: DigestBoundRef<"qa.process-launch-binding/v1">;
      executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
      target_process: ProcessIdentity;
    }
  | { kind: "secret_release"; lease_ref: DigestBoundRef<"qa.credential-lease/v1">; materialization_receipt_ref: DigestBoundRef<"qa.secret-materialization-receipt/v1"> }
  | { kind: "port_allocate"; protocol: "tcp"; bind_scope: "loopback"; requested_port?: number }
  | { kind: "directory_create"; path: RootQualifiedPath; mode: "0700" | "0750" }
  | { kind: "share_mount"; root: SandboxRootName; guest_path: string; mode: "read_only" | "read_write" }
  | { kind: "artifact_stage"; source: RootQualifiedPath; quarantine_id: string; media_type: string }
  | { kind: "vz_launch"; descriptor_ref: DigestBoundRef<"qa.vz-sandbox-descriptor/v1"> }
  | { kind: "vz_stop"; vm_resource_ref: DigestBoundRef; reason: TerminationReason }
);

type EffectAuthorization =
  | {
      kind: "grant_admission";
      grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
      authorization_input_digest: Sha256;
      admission_requirements_digest: Sha256;
      admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">;
    }
  | {
      kind: "recovery_admission";
      recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">;
      new_local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
      new_execution_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      authorization_input_digest: Sha256;
      admission_requirements_digest: Sha256;
      admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">;
    };

type EffectContextBase = ContractMeta & {
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  authorization: EffectAuthorization;
  fence: ExecutionFence;
  stable_environment_id: string;
  expected_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  expected_inventory_version: number;
  caller_process_identity?: ProcessIdentity;
};

type DesignBootstrapEffectContext = EffectContextBase & {
  kind: "design_bootstrap";
  design_policy_decision_ref: DigestBoundRef<"qa.design-policy-decision/v1">;
  design_scope_digest: Sha256;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
};

type DesignActiveEffectContext = EffectContextBase & {
  kind: "design_active";
  design_policy_decision_ref: DigestBoundRef<"qa.design-policy-decision/v1">;
  design_scope_digest: Sha256;
  environment_ref: DigestBoundRef<"qa.design-environment-receipt/v1">;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
};

type ExecutionBootstrapEffectContext = EffectContextBase & {
  kind: "execution_bootstrap";
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
  approved_envelope_digest: Sha256;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
};

type ExecutionStepEffectContext = EffectContextBase & {
  kind: "execution_step";
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  step: PlanStep;
  policy_decision_ref: DigestBoundRef<"qa.policy-decision/v1">;
  approved_envelope_digest: Sha256;
  environment_ref: DigestBoundRef<"qa.prepared-environment/v1">;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
};

type ControlQuiesceReconcileEffectContext = ContractMeta & {
  kind: "control_quiesce_reconcile";
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  fence: ExecutionFence;
  stable_environment_id: string;
  authority_ref: DigestBoundRef<"qa.cancellation-intent/v1" | "qa.timeout-intent/v1" | "qa.recovery-decision/v1">;
  open_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  open_inventory_version: number;
  process_warden_scope_ref?: DigestBoundRef<"qa.process-warden-scope/v1">;
  allowed_operations: Array<"quiesce" | "reconcile" | "terminate" | "revoke">;
};

type ControlCleanupEffectContext = ContractMeta & {
  kind: "control_cleanup";
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  fence: ExecutionFence;
  stable_environment_id: string;
  cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_version: number;
  resource_inventory_digest: Sha256;
  inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
  authority: OwnerCleanupAuthority | HigherFenceCleanupAuthority;
  process_warden_scope_ref?: DigestBoundRef<"qa.process-warden-scope/v1">;
};

type EffectContext =
  | DesignBootstrapEffectContext
  | DesignActiveEffectContext
  | ExecutionBootstrapEffectContext
  | ExecutionStepEffectContext
  | ControlQuiesceReconcileEffectContext
  | ControlCleanupEffectContext;

type DesignVerifierInput = {
  kind: "design";
  request: EffectRequest;
  context: DesignBootstrapEffectContext | DesignActiveEffectContext;
  design_grant: SignedGrant<DesignGrantClaims>;
  design_policy_decision: Extract<DesignPolicyDecision, { effect: "allow" }>;
};

type ExecutionVerifierInput = {
  kind: "execution";
  request: EffectRequest;
  context: ExecutionBootstrapEffectContext | ExecutionStepEffectContext;
  execution_grant: SignedGrant<ExecutionGrantClaims>;
  plan: StructuredPlan;
  policy_decision: Extract<PolicyDecision, { effect: "allow" }>;
};

type QuiesceReconcileVerifierInput = {
  kind: "control_quiesce_reconcile";
  request: EffectRequest;
  context: ControlQuiesceReconcileEffectContext;
};

type CleanupVerifierInput = {
  kind: "control_cleanup";
  request: EffectRequest;
  context: ControlCleanupEffectContext;
  cleanup_capability: CleanupCapability;
  inventory_seal_receipt: Extract<InventorySealReceipt, { kind: "sealed" }>;
};

type VerifierInput = DesignVerifierInput | ExecutionVerifierInput | QuiesceReconcileVerifierInput | CleanupVerifierInput;

type CheckedDigests =
  | { kind: "design"; request: Sha256; run_spec: Sha256; design_policy_decision: Sha256; design_scope: Sha256; grant: Sha256; effect_authorization: Sha256; admission_receipt: Sha256; fence: Sha256; inventory_snapshot: Sha256 }
  | { kind: "execution"; request: Sha256; run_spec: Sha256; plan: Sha256; plan_action: Sha256; policy_decision: Sha256; approved_envelope: Sha256; grant: Sha256; effect_authorization: Sha256; admission_receipt: Sha256; fence: Sha256; inventory_snapshot: Sha256 }
  | { kind: "control_quiesce_reconcile"; request: Sha256; authority_proof: Sha256; open_inventory_snapshot: Sha256; fence: Sha256 }
  | { kind: "control_cleanup"; request: Sha256; cleanup_capability: Sha256; inventory_snapshot: Sha256; inventory_seal_receipt: Sha256; authority_proof: Sha256; fence: Sha256 };

type EffectDecision =
  | {
      effect: "allow";
      decision_id: string;
      reason_codes: string[];
      checked_digests: CheckedDigests;
      obligations: string[];
    }
  | {
      effect: "deny";
      decision_id: string;
      reason_codes: [string, ...string[]];
      checked_digests: CheckedDigests;
      failed_constraint: "schema" | "phase" | "plan" | "policy" | "grant" | "fence" | "lease" | "admission_receipt" | "inventory" | "inventory_seal" | "warden" | "executable_identity" | "process_launch_binding" | "capability" | "recovery_proof";
    };

type EffectReceipt =
  | (ContractMeta & {
      kind: "performed";
      receipt_id: string;
      request: EffectRequest;
      context_digest: Sha256;
      decision: Extract<EffectDecision, { effect: "allow" }>;
      effect_state: "applied";
      sanitized_observation_ref?: DigestBoundRef<"qa.sanitized-observation/v1">;
      redaction_receipt_refs: DigestBoundRef<"qa.redaction-receipt/v1">[];
      dependency_acquisition_receipt_refs: DigestBoundRef<"qa.dependency-acquisition-receipt/v1">[];
      network_flow_receipt_refs: DigestBoundRef<"qa.network-flow-receipt/v1">[];
      resource_limit_receipt_refs: DigestBoundRef<"qa.resource-limit-receipt/v1">[];
      process_identities: ProcessIdentity[];
      secret_materialization_receipt_refs: DigestBoundRef<"qa.secret-materialization-receipt/v1">[];
      resource_usage_receipt_refs: DigestBoundRef<"qa.resource-usage-receipt/v1">[];
      previous_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      resulting_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      performed_at: ISO8601;
    })
  | (ContractMeta & {
      kind: "denied";
      receipt_id: string;
      request: EffectRequest;
      context_digest: Sha256;
      decision: Extract<EffectDecision, { effect: "deny" }>;
      effect_state: "denied";
      denied_at: ISO8601;
    })
  | (ContractMeta & {
      kind: "suppressed";
      receipt_id: string;
      request: EffectRequest;
      context_digest: Sha256;
      effect_state: "suppressed";
      suppression_authority:
        | { kind: "cancellation"; cancellation_intent_ref: DigestBoundRef<"qa.cancellation-intent/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "timeout"; timeout_intent_ref: DigestBoundRef<"qa.timeout-intent/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "revocation"; grant_revocation_receipt_ref: DigestBoundRef<"qa.grant-revocation-receipt/v1">; fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1"> }
        | { kind: "drain"; update_manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1"> };
      inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      suppressed_at: ISO8601;
    })
  | (ContractMeta & {
      kind: "failed";
      receipt_id: string;
      request: EffectRequest;
      context_digest: Sha256;
      decision: Extract<EffectDecision, { effect: "allow" }>;
      effect_state: "failed_retryable" | "failed_final" | "uncertain";
      side_effect_state: "none" | "partial" | "unknown";
      resource_usage_receipt_refs: DigestBoundRef<"qa.resource-usage-receipt/v1">[];
      resulting_inventory_snapshot_ref?: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      error: ErrorEnvelope;
      failed_at: ISO8601;
    });

type DesignEffectVerifier = { verify(input: DesignVerifierInput): Promise<EffectDecision> };
type ExecutionEffectVerifier = { verify(input: ExecutionVerifierInput): Promise<EffectDecision> };
type QuiesceReconcileEffectVerifier = { verify(input: QuiesceReconcileVerifierInput): Promise<EffectDecision> };
type CleanupEffectVerifier = { verify(input: CleanupVerifierInput): Promise<EffectDecision> };

type EffectGate = {
  perform(input: { request: EffectRequest; context: EffectContext }): Promise<EffectReceipt>;
};

type SecretBrokerHealth = RuntimeScopedMeta & {
  broker_instance_id: string;
  broker_boot_epoch: string;
  status: "healthy" | "degraded" | "recovering" | "unhealthy";
  accepting_requests: boolean;
  active_lease_count: number;
  unresolved_materialization_count: number;
  supported_modes: Array<"environment" | "file" | "proxy">;
  executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  checked_at: ISO8601;
  signature: SignatureBlock;
};

type SecretBrokerBinding = RuntimeScopedMeta & {
  broker_instance_id: string;
  broker_boot_epoch: string;
  broker_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  broker_process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
  local_ipc_binding_ref: DigestBoundRef<"qa.local-ipc-binding/v1">;
  audience: "fkst-local-secret-broker";
  supported_modes: Array<"environment" | "file" | "proxy">;
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type SecretBrokerRequestBase = ContractMeta & {
  broker_binding_ref: DigestBoundRef<"qa.secret-broker-binding/v1">;
  broker_boot_epoch: string;
  effect_record_ref: DigestBoundRef<"qa.effect-record/v1">;
  request_digest: Sha256;
  idempotency_key: string;
};

type SecretBrokerRequest =
  | (SecretBrokerRequestBase & {
      kind: "issue";
      secret_ref: string;
      environment_id: string;
      step_ids: string[];
      process_domain_refs: [DigestBoundRef<"qa.process-domain-descriptor/v1">, ...DigestBoundRef<"qa.process-domain-descriptor/v1">[]];
      process_launch_binding_refs: [DigestBoundRef<"qa.process-launch-binding/v1">, ...DigestBoundRef<"qa.process-launch-binding/v1">[]];
      executable_identity_refs: [DigestBoundRef<"qa.executable-identity/v1">, ...DigestBoundRef<"qa.executable-identity/v1">[]];
      inject_as: "environment" | "file" | "proxy";
      allowed_destinations: string[];
      fence_digest: Sha256;
    })
  | (SecretBrokerRequestBase & {
      kind: "materialize";
      lease_ref: DigestBoundRef<"qa.credential-lease/v1">;
      target_process: ProcessIdentity;
      process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
      process_launch_binding_ref: DigestBoundRef<"qa.process-launch-binding/v1">;
      executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
    })
  | (SecretBrokerRequestBase & { kind: "release"; materialization_receipt_ref: DigestBoundRef<"qa.secret-materialization-receipt/v1">; reason: CleanupReason })
  | (SecretBrokerRequestBase & { kind: "revoke"; lease_ref: DigestBoundRef<"qa.credential-lease/v1">; reason: CleanupReason })
  | (SecretBrokerRequestBase & { kind: "reconcile"; lease_ref: DigestBoundRef<"qa.credential-lease/v1"> });

type SecretBrokerClient = {
  execute(request: SecretBrokerRequest): Promise<CredentialLeaseReceipt | SecretMaterializationReceipt>;
};
```

`VerifierInput`、`EffectRequest`、`EffectAuthorization`、`EffectContext`、`CheckedDigests`、`EffectDecision` 和 `EffectReceipt` 都是 exact-object strict union。Design verifier 只能接受 Design context/Grant/Policy，Execution verifier 只能接受 Execution context/Grant/Plan/Policy，Cleanup verifier 只能接受 control-cleanup context/capability/sealed inventory；跨 phase 字段必须以 `contract.forbidden_field` 拒绝。`design_bootstrap` 与 `execution_bootstrap` 明确禁止要求或携带 `PlanStep`、`PreparedEnvironment`、active worker identity 或尚未创建的 ProcessWardenScope；它们只能执行创建对应 environment 所需、且已被 admission receipt/Grant/Policy/initial inventory/CleanupCapability 绑定的最小 bootstrap effect。每个文件、进程、网络、Secret、浏览器和资源动作必须由 Rust `EffectGate.perform` 统一完成 admission、执行、inventory 更新和 Receipt 持久化。`EffectGate` 禁止向 TypeScript worker、guest agent 或 Backend 返回可脱离 Gate 使用的 allow token、文件描述符、CDP endpoint、Secret material 或裸宿主 capability；`EffectDecision(effect="allow")` 只能作为同一 `perform` 调用内部及其 Receipt 的组成部分。

`EffectGate.perform` 必须先在 SQLite 写入 `EffectRecord(state="pending")`，再原子转为 `dispatching` 后调用受控 adapter，并按 §16.2 的 canonical EffectState 写入 Receipt、inventory 新版本和 Event outbox。deny 必须在副作用前持久化；`failed_retryable` 只能在可证明无副作用或有确定补偿时重试，`failed_final` 不得重放，`uncertain` 必须进入 `reconciling`。`side_effect_state="partial"|"unknown"` 必须携带或随后生成可 Cleanup 的 inventory snapshot，并强制 reconcile/seal/cleanup，禁止由 caller 自行猜测是否重试。所有 adapter 原始输出先进入本地 raw quarantine；只有持久化 RedactionReceipt 和 SanitizedObservation 后才能进入普通 Receipt/Event/Artifact。

Secret Broker 是与 Supervisor 同一签名部署目标内、由 Warden 管理的独立非特权 helper。Supervisor/EffectGate 决定是否允许 Secret effect，并只向 broker 发送 exact `SecretBrokerRequest`；broker 不能读取 Plan、签发 Grant、扩大 scope 或写 Ledger。broker 必须验证自身 binding/boot epoch、EffectRecord、Grant-derived binding、Step/destination/fence、ProcessDomainDescriptor 与实际 ProcessIdentity。proxy mode 下只有 broker 是明文 custodian；guest injection 下 broker 与受控 injector/获准 process domain 是临时 custodian；environment/file mode 必须把获准 descendants、FD/environment/file inheritance 和擦除范围写入 ProcessDomainDescriptor 与 Receipt。所有模式都必须禁用 core dump，阻止 domain 外 ptrace，避免 swap/crash report/logging 泄漏。Step 完成、取消、超时、Grant 撤销、Cleanup 或 Runtime 恢复时必须 release materialization并 revoke/reconcile lease；状态未知时形成 blocking residual。

### 9.5 Sandbox 强制要求

EnvironmentFactory 必须：

- 创建每 Run 独立的 Workspace、VZ VM identity、ProcessWardenScope、ResourceInventorySnapshot lineage 和 CleanupCapability。
- 从 SourceObject 获取并验证 `effective_sha` 与对象 digest，禁止执行浮动 branch pull 或信任 `clone_url`。
- 默认不挂载用户主目录、SSH 目录、浏览器个人 Profile、系统 Keychain 或其他项目目录。
- 只把 Grant 和 Plan 允许的路径以最小只读/读写模式共享到 Linux guest；所有 host/guest path token 和 mount 必须由 EffectGate 重新判定。
- 使用宿主系统 Google Chrome 和独立临时 Profile；Run 结束后必须由 host Process Warden 终止并删除 Profile，禁止 preserve 浏览器 Profile。
- 对 VM、guest agent、VM 内 TypeScript worker/App/Middleware/Backend、受控 host helper 和 host Chrome 使用同一 Run ownership domain，并用可跨 host/guest 对账的 ProcessIdentity 终止。
- 显式登记进程、端口、临时文件、VZ VM、mount、CredentialLease、Secret materialization、Browser Profile 和 Artifact staging resource，每次变化产生 inventory 新版本。
- TypeScript worker 只能经与 `VZSandboxDescriptor`、`GuestBootEvidence`、guest boot、guest agent ExecutableIdentity、bootstrap nonce、ephemeral key 和 channel transcript digest 完整绑定的 `BootBoundAuthenticatedVsockSession` 请求 `EffectGate.perform`；禁止直接调用 `spawn`、宿主 filesystem、network socket、Chrome automation endpoint、Secret Broker materialization 或 SQLite writer Interface。
- Node/Adapter、本地 CLI、BrowserProvider 和受控 helper 只能经有效 `LocalIPCBinding` 访问 Runtime；Runtime 必须校验 Unix peer credential 或 loopback mTLS、调用方 ExecutableIdentity、audience、protocol、TTL 和 nonce，禁止仅因连接来自 loopback 就信任。
- 所有 process spawn 必须先冻结 `ProcessLaunchBinding` 并验证 ExecutableIdentity；所有 Secret materialization 必须绑定该 launch binding 与实际 ProcessIdentity。
- 在取消或超时时终止完整资源域，而不是只杀父进程。
- 允许 Cleanup 在 prepare 部分完成、Runtime 重启或云端断线后凭 capability 与最新 sealed inventory snapshot 单独重试。

---

## 10. `TestingBackend`、BrowserProvider 与 Runner Assertion

### 10.1 Backend Interface 与 TerminationReceipt

```ts
type BackendObservation = ContractMeta & {
  observation_id: string;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  step_id: string;
  attempt: number;
  backend: "deterministic" | "browser" | "codex";
  started_at: ISO8601;
  completed_at: ISO8601;
  exit?: { code?: number; signal?: string };
  stdout_ref?: DigestBoundRef<"qa.artifact-pointer/v1">;
  stderr_ref?: DigestBoundRef<"qa.artifact-pointer/v1">;
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  artifact_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  redaction_receipt_refs: DigestBoundRef<"qa.redaction-receipt/v1">[];
  backend_error?: ErrorEnvelope;
  amendment_signal?: { reason_code: AmendmentReasonCode; requested_action: PlanAction };
};

type StepAttemptReceipt = ContractMeta & {
  receipt_id: string;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  step_id: string;
  attempt: number;
  backend: "deterministic" | "browser" | "codex";
  fence: ExecutionFence;
  effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
  observation_ref?: DigestBoundRef<"qa.backend-observation/v1">;
  termination_receipt_ref?: DigestBoundRef<"qa.termination-receipt/v1">;
  resource_usage_receipt_refs: DigestBoundRef<"qa.resource-usage-receipt/v1">[];
  outcome: "completed" | "failed" | "cancelled" | "timed_out" | "amendment_required";
  started_at: ISO8601;
  completed_at: ISO8601;
  error?: ErrorEnvelope;
};

type TerminationReason = "cancelled" | "timed_out" | "amendment" | "grant_revoked" | "shutdown" | "cleanup" | "recovery" | "repair";

type TerminationTargetScope =
  | { kind: "step_attempt"; environment_id: string; step_id: string; attempt: number }
  | { kind: "environment"; environment_id: string }
  | { kind: "process_domain"; environment_id: string; process_group_identity: string }
  | { kind: "resource_set"; environment_id: string; inventory_resource_refs: [DigestBoundRef, ...DigestBoundRef[]] }
  | { kind: "runtime_shutdown"; runtime_instance_id: string; environment_ids: string[] };

type TerminationReceipt =
  | (ContractMeta & {
      kind: "performed";
      receipt_id: string;
      target_scope: TerminationTargetScope;
      requested_reason: TerminationReason;
      requested_at: ISO8601;
      completed_at: ISO8601;
      process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
      target_process_identities: ProcessIdentity[];
      terminated_process_identity_refs: DigestBoundRef<"qa.process-identity/v1">[];
      resource_usage_receipt_refs: DigestBoundRef<"qa.resource-usage-receipt/v1">[];
      outcome: "terminated" | "partially_terminated" | "failed";
      remaining_resource_refs: DigestBoundRef[];
      error?: ErrorEnvelope;
    })
  | (ContractMeta & {
      kind: "already_terminated";
      receipt_id: string;
      target_scope: TerminationTargetScope;
      requested_reason: TerminationReason;
      prior_termination_receipt_ref?: DigestBoundRef<"qa.termination-receipt/v1">;
      reconciled_effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
      inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      completed_at: ISO8601;
      outcome: "already_terminated";
    });

type TestingBackend = {
  execute(input: {
    run_id: UUID;
    step: PlanStep;
    environment: PreparedEnvironment;
    policy_decision: PolicyDecision;
    grant: SignedGrant<ExecutionGrantClaims>;
    fence: ExecutionFence;
    attempt: number;
  }): Promise<BackendObservation>;
  cancel(input: {
    run_id: UUID;
    target_scope: TerminationTargetScope;
    reason: TerminationReason;
    fence: ExecutionFence;
    deadline_at: ISO8601;
    idempotency_key: string;
  }): Promise<TerminationReceipt>;
};
```

Deterministic、Browser 和 Codex Backend 必须实现同一 observation 接口。Backend/TypeScript worker 必须把动作建模为 EffectRequest 并调用 `EffectGate.perform`；禁止在取得 allow decision 后自行执行。EffectGate 必须校验 phase-specific context、Plan action/envelope（Execution）、Policy、Grant、lease/fence、admission receipt、inventory version、ExecutableIdentity、ProcessLaunchBinding 和 Process Warden identity。`cancel` 的 acknowledgement 不是完成证据；只有 `TerminationReceipt(kind="performed", outcome="terminated")` 或可验证的 `kind="already_terminated"` 才能证明 target scope 已终止，`partially_terminated`/`failed` 必须进入 Cleanup/repair，且任何 TerminationReceipt 都不能替代其他 inventory resource 的 Cleanup。

### 10.2 BrowserProvider 与最小安全能力集

```ts
type NetworkProxyBinding = ContractMeta & {
  binding_id: string;
  environment_id: string;
  enforcement_mechanism: "process_tree_socket_filter" | "network_extension" | "equivalent_os_enforcer";
  enforcer_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  enforcer_process_identity_ref: DigestBoundRef<"qa.process-identity/v1">;
  enforcer_version: string;
  configuration_digest: Sha256;
  root_process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
  covered_process_identity_refs: DigestBoundRef<"qa.process-identity/v1">[];
  protocols: Array<"ipv4_tcp" | "ipv4_udp" | "ipv6_tcp" | "ipv6_udp">;
  proxy_endpoint_token: string;
  dns_mediation: "enforced";
  direct_socket_policy: "deny";
  established_at: ISO8601;
  expires_at: ISO8601;
};

type BrowserEnforcementCapability = RuntimeScopedMeta & {
  capability_id: string;
  probe_version: string;
  mechanism: NetworkProxyBinding["enforcement_mechanism"];
  enforcer_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  supported_protocols: NetworkProxyBinding["protocols"];
  covers_dynamic_chrome_descendants: true;
  quic_denied_or_mediated: true;
  webrtc_udp_denied_or_mediated: true;
  dns_and_doh_policy_digest: Sha256;
  probe_suite_digest: Sha256;
  probe_result_digest: Sha256;
  valid_from: ISO8601;
  valid_through: ISO8601;
  signature: SignatureBlock;
};

type BrowserNetworkEnforcementReceipt = ContractMeta & {
  receipt_id: string;
  environment_id: string;
  session_id: string;
  capability_ref: DigestBoundRef<"qa.browser-enforcement-capability/v1">;
  forced_proxy_binding_ref: DigestBoundRef<"qa.network-proxy-binding/v1">;
  chrome_root_process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
  covered_process_tree_digest: Sha256;
  enforcer_identity_digest: Sha256;
  enforcement_configuration_digest: Sha256;
  network_policy_digest: Sha256;
  approved_destination_set_digest: Sha256;
  direct_socket_denied: true;
  dns_resolution_mediated: true;
  allowed_connection_set_digest: Sha256;
  denied_connection_set_digest: Sha256;
  enforcement_loss_events: [];
  started_at: ISO8601;
  observed_through: ISO8601;
  signature: SignatureBlock;
};

type BrowserSession = ContractMeta & {
  session_id: string;
  environment_id: string;
  browser: "host_system_google_chrome";
  automation_channel: "provider_mediated";
  profile_ref: DigestBoundRef;
  process_identity: ProcessIdentity;
  process_domain_ref: DigestBoundRef<"qa.process-domain-descriptor/v1">;
  process_launch_binding_ref: DigestBoundRef<"qa.process-launch-binding/v1">;
  process_warden_scope_ref: DigestBoundRef<"qa.process-warden-scope/v1">;
  forced_proxy_binding_ref: DigestBoundRef<"qa.network-proxy-binding/v1">;
  network_enforcement_receipt_ref: DigestBoundRef<"qa.browser-network-enforcement-receipt/v1">;
  capabilities: BrowserSecurityCapability[];
};

type BrowserSecurityCapability =
  | "isolated_profile"
  | "ephemeral_profile"
  | "download_directory_isolation"
  | "network_policy_enforcement"
  | "forced_proxy"
  | "direct_socket_denial"
  | "origin_allowlist"
  | "permission_prompt_control"
  | "credential_store_disabled"
  | "extension_isolation"
  | "process_group_termination"
  | "artifact_redaction";

type BrowserActionReceipt = ContractMeta & {
  receipt_id: string;
  action: BrowserAction;
  action_digest: Sha256;
  session_ref: DigestBoundRef<"qa.browser-session/v1">;
  effect_receipt_ref: DigestBoundRef<"qa.effect-receipt/v1">;
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  artifact_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  network_enforcement_receipt_ref: DigestBoundRef<"qa.browser-network-enforcement-receipt/v1">;
  termination_receipt_ref?: DigestBoundRef<"qa.termination-receipt/v1">;
  completed_at: ISO8601;
};

type BrowserProvider = {
  performAction(input: {
    action: BrowserAction;
    execution_grant: SignedGrant<ExecutionGrantClaims>;
    effect_context: ExecutionBootstrapEffectContext | ExecutionStepEffectContext;
    idempotency_key: string;
  }): Promise<BrowserActionReceipt>;
};
```

Browser Step 至少必须要求 `isolated_profile`、`ephemeral_profile`、`download_directory_isolation`、`network_policy_enforcement`、`forced_proxy`、`direct_socket_denial`、`origin_allowlist`、`credential_store_disabled`、`extension_isolation` 和 `process_group_termination`。Runtime 只有在当前、未过期的 `BrowserEnforcementCapability` 证明整个 Chrome process tree 的 IPv4/IPv6 TCP/UDP、QUIC/WebRTC 与动态 network-service descendants 均被 OS-level 或等价不可绕过机制中介时，才能在 RuntimeHealth/AdmissionSnapshot 广告 Browser capability；否则必须在 reservation 前拒绝 Browser Plan，非 Browser Plan 可继续。每个 Session 必须持久化签名 `BrowserNetworkEnforcementReceipt`，不能用 Runtime 自报布尔值代替 enforcer identity、配置、process coverage 和 allow/deny telemetry。运行中 enforcement 丢失时必须先 deny network，再终止完整 Chrome process domain并隔离 raw output，禁止降级为仅 Chrome flags、个人 Profile、软 allowlist 或无网络约束 Chrome。

`BrowserSession` 是 opaque authority record：hosted、NyxID、Plan、RuntimeEvent 和 TypeScript worker 只能持有 `DigestBoundRef<"qa.browser-session/v1">`。CDP/WebSocket/debugging port、endpoint、token、file descriptor 和 provider internal handle 禁止出现在该对象或任何跨进程 contract。BrowserProvider 必须是 EffectGate 后方的 Rust-owned adapter；所有 strict `BrowserAction` 只能经 `BrowserProvider.performAction`，该方法内部调用 `EffectGate.perform` 并解析 automation channel。Capture 的原始 DOM、trace、network body 和 screenshot 必须先进入 raw quarantine，完成 RedactionReceipt 后才返回 sanitized observation 或 post-redaction ArtifactPointer。

### 10.3 Assertion Evaluator

```ts
type AssertionResult = {
  assertion_id: string;
  assertion_type: AssertionSpec["type"];
  required: boolean;
  outcome: "passed" | "failed" | "not_evaluated" | "error";
  expected?: unknown;
  actual?: unknown;
  reason_code?: string;
  observation_ref: DigestBoundRef<"qa.backend-observation/v1">;
  evidence_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
};

type RunnerAssertionEvaluator = {
  evaluate(input: { step: PlanStep; observation: BackendObservation }): Promise<AssertionResult[]>;
};
```

判定规则：

1. Backend protocol error 必须使受影响 Case 至少为 `error`，禁止判定为 passed。
2. required assertion 必须按 §6.2 聚合；optional assertion 失败必须记录但不得独自使 Case failed，除非 Case 策略另有明确规则。
3. assertion 无法执行且无允许的 fallback 时，Case 必须为 inconclusive 或 error，禁止默认通过。
4. Codex 生成的解释可以作为 Evidence，但不得作为唯一 assertion actual。

### 10.4 CaseResult

```ts
type CaseResult = ContractMeta & {
  case_result_id: string;
  case_id: string;
  suite_id?: string;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  step_ids: string[];
  assertion_ids: string[];
  evidence_requirement_ids: string[];
  outcome: "passed" | "failed" | "error" | "skipped" | "inconclusive";
  assertion_results: AssertionResult[];
  attempts_by_step: Record<string, number>;
  duration_ms: number;
  observation_refs: DigestBoundRef<"qa.backend-observation/v1">[];
  evidence_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  skip_reason?:
    | { kind: "not_applicable"; condition_digest: Sha256; evaluated_inputs_digest: Sha256 }
    | { kind: "cancelled" | "timed_out"; reason_code: string };
  failure_hint?: {
    classification_candidates: FailureClassification[];
    reason_codes: string[];
  };
};
```

CaseResult 必须包含 PlanCase 声明的全部 assertion id 和 evidence requirement id；不得附加其他 Case 的结果。`outcome="skipped"` 必须同时携带 `skip_reason`：`not_applicable` 必须引用 Plan 中的 ApplicabilityCondition digest 与确定性输入摘要，取消/超时必须记录 reason code。非 skipped Case 禁止携带 `skip_reason`，禁止把未执行 Case 伪装为 passed。

---

## 11. Artifact、Evidence 与 Cleanup 契约

### 11.1 ArtifactPointer

```ts
type RedactionRule =
  | { kind: "header"; names_lowercase: string[]; action: "remove" | "replace_constant"; replacement?: string }
  | { kind: "cookie"; names: string[] | "all"; action: "remove" | "replace_constant"; replacement?: string }
  | { kind: "json_path"; paths: string[]; value_types: Array<"string" | "number" | "boolean" | "object" | "array" | "null">; action: "remove" | "replace_constant" | "hmac_sha256"; replacement?: string }
  | { kind: "text_pattern"; engine: "re2"; pattern: string; maximum_matches: number; action: "replace_constant" | "hmac_sha256"; replacement?: string }
  | { kind: "filesystem_path"; allowed_root_names: SandboxRootName[]; action: "replace_with_root_qualified_path" | "remove" }
  | { kind: "image_region"; detector_ref: DigestBoundRef<"qa.redaction-detector/v1">; action: "solid_fill"; fill_rgb: [number, number, number] };

type RedactionPolicy = ContractMeta & {
  policy_id: string;
  version: string;
  allowed_input_media_types: string[];
  allowed_output_media_types: string[];
  rules: [RedactionRule, ...RedactionRule[]];
  rule_set_digest: Sha256;
  hmac_key_ref?: string; // opaque Runtime redactor key reference；禁止进入 Receipt/Event/Artifact
  limits: {
    maximum_input_bytes: number;
    maximum_output_bytes: number;
    maximum_findings: number;
    maximum_elapsed_millis: number;
    maximum_decompression_ratio: number;
    maximum_archive_entries: number;
  };
  output_validation: {
    forbidden_secret_classes: Array<"authorization_header" | "cookie" | "access_token" | "private_key" | "keychain_value" | "absolute_user_path" | "unapproved_personal_data">;
    required_schema_refs: DigestBoundRef[];
    require_second_pass_scan: true;
  };
  failure_behavior: "retain_encrypted_quarantine_until_ttl_or_delete";
  redactor_executable_identity_refs: [DigestBoundRef<"qa.executable-identity/v1">, ...DigestBoundRef<"qa.executable-identity/v1">[]];
};

type RawQuarantineArtifact = ContractMeta & {
  quarantine_id: string;
  environment_id: string;
  producing_effect_id: string;
  producing_effect_request_digest: Sha256;
  media_type: string;
  raw_byte_size: number;
  raw_byte_digest: Sha256;
  local_opaque_storage_token: string;
  access: "runtime_redactor_only";
  expires_at: ISO8601;
};

type RedactionReceipt =
  | (ContractMeta & {
      kind: "completed";
      receipt_id: string;
      quarantine_ref: DigestBoundRef<"qa.raw-quarantine-artifact/v1">;
      redaction_policy_ref: DigestBoundRef<"qa.redaction-policy/v1">;
      redactor_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
      rule_set_digest: Sha256;
      raw_byte_digest: Sha256;
      raw_bytes_scanned: number;
      sanitized_byte_digest: Sha256;
      sanitized_byte_size: number;
      findings_count: number;
      transformations_digest: Sha256;
      second_pass_scan_digest: Sha256;
      output_schema_validation_digest: Sha256;
      quarantine_disposition: "deleted" | "retained_encrypted_until_ttl";
      completed_at: ISO8601;
      signature: SignatureBlock;
    })
  | (ContractMeta & {
      kind: "failed";
      receipt_id: string;
      quarantine_ref: DigestBoundRef<"qa.raw-quarantine-artifact/v1">;
      redaction_policy_ref: DigestBoundRef<"qa.redaction-policy/v1">;
      rule_set_digest: Sha256;
      raw_byte_digest: Sha256;
      raw_bytes_scanned: number;
      quarantine_disposition: "deleted" | "retained_encrypted_until_ttl";
      error: ErrorEnvelope;
      failed_at: ISO8601;
      signature: SignatureBlock;
    });

type SanitizedObservation = ContractMeta & {
  observation_id: string;
  producing_effect_id: string;
  producing_effect_request_digest: Sha256;
  redaction_receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1">;
  schema_ref: DigestBoundRef;
  sanitized_payload: unknown;
  sanitized_payload_digest: Sha256;
  persisted_at: ISO8601;
};

type ArtifactRetentionClass =
  | "raw_quarantine"
  | "sanitized_evidence"
  | "publication_rendered"
  | "runtime_diagnostic";

type ArtifactEncryptionEnvelope = {
  algorithm: "AES-256-GCM";
  key_management: "runtime_keychain_wrapped_dek" | "provider_kms_wrapped_dek";
  wrapped_dek_ref: string;            // opaque ref；禁止包含 key material
  nonce_base64url: Base64UrlNoPad;    // 每个 object 唯一 96-bit nonce
  aad_digest: Sha256;                 // 绑定 artifact/run/media_type/byte_digest
};

type ArtifactRetentionPolicy = {
  schema_version: "qa.artifact-retention-policy/v1";
  content_digest: Sha256;
  policy_id: string;
  version: string;
  profiles: Record<ArtifactRetentionClass, {
    default_ttl_seconds: number;
    maximum_ttl_seconds: number;
    allow_legal_hold: boolean;
  }>;
};

type ArtifactDeletionReceipt = ContractMeta & {
  deletion_receipt_id: string;
  artifact_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
  retention_policy_ref: DigestBoundRef<"qa.artifact-retention-policy/v1">;
  disposition: "deleted" | "already_absent" | "legal_hold_active" | "failed";
  access_revoked_at?: ISO8601;
  object_deleted_at?: ISO8601;
  wrapped_dek_destroyed_at?: ISO8601;
  verified_absent_at?: ISO8601;
  error?: ErrorEnvelope;
  settled_at: ISO8601;
};

type ArtifactPointer = ContractMeta & {
  artifact_id: string;
  media_type: string;
  byte_size: number;
  byte_digest: Sha256;                // 必须是 post-redaction bytes digest
  digest_stage: "post_redaction";
  redaction_receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1">;
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  storage:
    | {
        type: "local";
        runtime_instance_id: string;
        opaque_path_token: string;
        encryption: ArtifactEncryptionEnvelope;
      }
    | {
        type: "encrypted_object";
        provider: string;
        object_key: string;
        encryption: ArtifactEncryptionEnvelope;
      }
    | {
        type: "inline";
        encoding: "base64";
        data: string;
        inline_class: "non_sensitive_rendered_summary";
      };
  access_scope: {
    readers: string[];
    capability_required: true;
    maximum_capability_ttl_seconds: 900;
  };
  retention: {
    class: Exclude<ArtifactRetentionClass, "raw_quarantine">;
    policy_ref: DigestBoundRef<"qa.artifact-retention-policy/v1">;
    delete_after: ISO8601;
    legal_hold: false | { hold_id: string; authority_ref: DigestBoundRef; expires_at?: ISO8601 };
  };
};
```

Raw bytes、DOM、trace、network payload、stdout/stderr 和 screenshot 必须先作为 `RawQuarantineArtifact` 进入 Runtime-only 隔离区；该对象及其 storage token 禁止经 RuntimeService、Event、hosted、NyxID、Backend 或 Publication 暴露。只有 `RedactionReceipt(kind="completed")` 精确绑定 raw digest、policy 和 sanitized digest 后才能创建 durable `SanitizedObservation` 与 `ArtifactPointer`。`ArtifactPointer.byte_digest` 必须按脱敏后原始 bytes 计算并等于 receipt 的 `sanitized_byte_digest`；失败 receipt 必须销毁或继续隔离 raw object，并阻止 Evidence sufficient、上传和发布。`opaque_path_token` 禁止暴露用户真实绝对路径。

`RedactionPolicy` 必须是 Runtime redactor 可直接执行的 exact contract，禁止只包含自然语言提示、示例或“best effort”开关。每条规则必须使用固定 engine/action，RE2 pattern、JSON path、header/cookie 名称、detector ref、替换常量和所有 limit 都属于 `rule_set_digest`；需要 HMAC pseudonymization 时只能传 opaque `hmac_key_ref` 给获准 redactor domain。输入 media type、byte/decompression/archive/time/finding limit 任一超限，unknown media type、detector unavailable、规则无法执行、second-pass 仍命中 forbidden class、output schema 不通过或 sanitized bytes 超限时必须产生 failed Receipt，不得输出部分脱敏 Artifact。Completed Receipt 必须证明 exact redactor identity、完整扫描字节数、transform digest、second-pass digest、schema validation digest 和 quarantine disposition；两种 Receipt 都必须由当前 device-bound Runtime key 按 §3.6 `purpose="redaction_receipt"` 签名。调用方禁止跳过规则、把 findings 截断后声称成功，或在 Receipt 持久化前释放 sanitized bytes。

v1 的 Artifact policy 固定如下：

1. `raw_quarantine` 只能保存在 Runtime 本地加密隔离区，默认 TTL 为 1 小时、最大 TTL 为 24 小时且不允许 legal hold；成功脱敏、失败、取消或超时后应尽快删除，TTL 只是崩溃恢复上限。
2. `sanitized_evidence` 默认 TTL 为 30 天、最大 TTL 为 90 天；`publication_rendered` 默认 TTL 为 90 天、最大 TTL 为 365 天；`runtime_diagnostic` 默认 TTL 为 7 天、最大 TTL 为 30 天。部署策略可以选择更短 TTL；延长到 maximum 之内必须使用新的 policy version，超过 maximum 只能使用显式 legal hold。
3. local 与 object storage 必须使用每个 Artifact 独立的随机 256-bit DEK 和 AES-256-GCM envelope encryption。Local DEK 由 Runtime 专用 Keychain wrapping key 包装；object DEK 由 provider KMS wrapping key 包装。Nonce 每个 object 唯一，AAD 必须绑定 `run_id`、`artifact_id`、`media_type` 和 `byte_digest`。Pointer、Ledger、Event 和日志禁止包含明文 DEK、wrapping key 或可直接解密的 credential。
4. `ArtifactPointer` 只是定位与完整性对象，不授予读取权限。每次读取必须验证短期 `ArtifactAccessCapability`；capability TTL 不得超过 15 分钟，必须绑定单一 Artifact、subject、必需的 Runtime audience、可选且唯一的 Store provider audience、allowed range、nonce 和 revocation id。RuntimeService range 或 Store byte range 必须完全落在 `allowed_ranges` 内。Grant/Run/用户访问撤销、legal hold 权限变化或主体权限下降时必须立即拒绝后续读取，禁止依赖 Pointer 的旧 `readers` 快照。
5. `inline` 只允许不含 Secret、cookie、token、原始 DOM、trace、network payload、stdout/stderr 或 screenshot 的脱敏渲染摘要，decoded 大小不得超过 32 KiB；其他 Artifact 必须使用加密 local/object storage。
6. 到达 `delete_after` 时先撤销 access capability，再删除 object/local bytes 和 wrapped DEK，并生成幂等 `ArtifactDeletionReceipt`。`already_absent` 视为删除已收敛；`legal_hold_active` 必须保留 hold authority；`failed` 必须进入 repair backlog。删除证明本身只保留 digest、时间和 disposition，不保留 object key、opaque path 或 key material。

### 11.2 EvidenceManifest

```ts
type EvidenceRequirementFulfillment =
  | {
      status: "fulfilled";
      requirement_id: string;
      artifact_refs: [DigestBoundRef<"qa.artifact-pointer/v1">, ...DigestBoundRef<"qa.artifact-pointer/v1">[]];
      assertion_ids: string[];
      fulfilled_at: ISO8601;
    }
  | {
      status: "missing";
      requirement_id: string;
      required: boolean;
      reason_code: string;
    }
  | {
      status: "failed";
      requirement_id: string;
      required: boolean;
      error: ErrorEnvelope;
    }
  | {
      status: "not_required";
      requirement_id: string;
      required: false;
      reason_code: "case_not_applicable" | "step_not_executed_by_plan";
    };

type EvidenceManifest = ContractMeta & {
  manifest_id: string;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  case_result_refs: DigestBoundRef<"qa.case-result/v1">[];
  requirement_fulfillments: EvidenceRequirementFulfillment[];
  artifacts: Array<{
    pointer: ArtifactPointer;
    requirement_ids: string[];
    type: EvidenceRequirement["type"];
    step_ids: string[];
    case_ids: string[];
    assertion_ids: string[];
    redaction: {
      status: "completed";
      policy_ref: DigestBoundRef<"qa.redaction-policy/v1">;
      receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1">;
      sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
      findings_count: number;
      post_redaction_digest: Sha256;
    };
  }>;
  evidence_outcome: "sufficient" | "partial" | "insufficient";
};
```

Manifest 必须对 Plan 中每个 EvidenceRequirement 恰好有一项 fulfillment，禁止遗漏、重复或引用未知 requirement。required requirement 只有 `fulfilled` 才满足；`missing`、`failed` 或非法 `not_required` 必须使 Evidence 非 sufficient。Artifact 必须反向列出所满足 requirement/case/step/assertion，且这些关联必须与 Plan 一致。

Secret、Authorization header、cookie、access token、Keychain 数据、用户目录路径和未批准个人数据必须在上传和发布前脱敏。Manifest 中每个 Artifact 的 redaction receipt、post-redaction digest 和 ArtifactPointer.byte_digest 必须一致；脱敏失败、raw quarantine 未结算或 durable SanitizedObservation 缺失时 Artifact 不得进入 Publication allowlist，Evidence 不得标记为 sufficient。

### 11.3 CleanupReceipt、Residual 与 Preserved Resource

```ts
type CleanupReason =
  | "design_completed"
  | "design_failed"
  | "execution_completed"
  | "execution_failed"
  | "cancelled"
  | "timed_out"
  | "runtime_lost"
  | "runtime_restart_recovery"
  | "control_cleanup_takeover"
  | "amendment_pause"
  | "update_drain"
  | "uninstall"
  | "manual_repair";

type CleanupResidualReason =
  | "resource_still_active"
  | "resource_identity_mismatch"
  | "resource_missing_without_proof"
  | "resource_state_unknown"
  | "inventory_entry_unresolvable"
  | "termination_unconfirmed"
  | "credential_revocation_unconfirmed"
  | "secret_release_unconfirmed"
  | "browser_proxy_detach_unconfirmed"
  | "mount_detach_failed"
  | "vm_stop_unconfirmed"
  | "artifact_quarantine_delete_failed"
  | "permission_denied"
  | "capability_scope_insufficient"
  | "seal_or_lineage_mismatch"
  | "retry_budget_exhausted";

type CleanupResidual = {
  resource: ResourceRef;
  category: InventoryResource["category"] | "unknown_inventory_resource";
  attempted_action: InventoryResource["cleanup_action"];
  reason_code: CleanupResidualReason;
  discovery_source: "inventory" | "warden_reconcile" | "effect_reconcile" | "secret_broker" | "browser_provider";
  exposure: "none" | "local_only" | "network_reachable" | "credential_active" | "secret_materialized" | "unknown";
  retryable: boolean;
  repair_required: boolean;
  last_error?: ErrorEnvelope;
};

type PreservedResource = {
  resource: ResourceRef;
  preservation_reason: "artifact_retention" | "audit_hold" | "user_requested" | "forensic_hold";
  retention_policy_ref: DigestBoundRef<"qa.retention-policy/v1">;
  delete_after?: ISO8601;
  custodian: ActorRef;
};

type PerformedCleanupReceipt = ContractMeta & {
  kind: "performed";
  receipt_id: string;
  cleanup_command_ref: DigestBoundRef<"qa.runtime-command/v1">;
  command_admission_receipt_ref: DigestBoundRef<"qa.command-admission-receipt/v1">;
  fence: ExecutionFence;
  authority_proof_ref: DigestBoundRef<"qa.local-execution-lease/v1" | "qa.recovery-decision/v1" | "qa.repair-operation/v1">;
  capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
  capability_sequence: number;
  predecessor_cleanup_receipt_ref?: DigestBoundRef<"qa.cleanup-receipt/v1">;
  cleanup_lineage_id: string;
  cleanup_attempt: number;
  cleanup_request_digest: Sha256;
  resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  resource_inventory_id: string;
  resource_inventory_lineage_id: string;
  resource_inventory_version: number;
  resource_inventory_digest: Sha256;
  inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
  reason: CleanupReason;
  started_at: ISO8601;
  completed_at: ISO8601;
  resources: Array<{
    resource: ResourceRef;
    action: "terminate" | "delete" | "revoke" | "release" | "preserve";
    outcome: "succeeded" | "failed" | "not_found" | "skipped";
    reason_code?: CleanupResidualReason | "already_settled_with_proof" | "preserved_by_policy";
    supporting_receipt_refs: DigestBoundRef[];
    retryable: boolean;
    error?: ErrorEnvelope;
  }>;
  credential_receipt_refs: DigestBoundRef<"qa.credential-lease-receipt/v1">[];
  secret_materialization_receipt_refs: DigestBoundRef<"qa.secret-materialization-receipt/v1">[];
  termination_receipt_refs: DigestBoundRef<"qa.termination-receipt/v1">[];
  resource_usage_receipt_refs: DigestBoundRef<"qa.resource-usage-receipt/v1">[];
  residuals: CleanupResidual[];
  preserved_resources: PreservedResource[];
  outcome: "succeeded" | "partially_succeeded" | "failed";
  next_retry_at?: ISO8601;
};

type NotRequiredCleanupReceipt = ContractMeta & {
  kind: "not_required";
  receipt_id: string;
  authority: "fkst-hosted.workflow" | "fkst-local-qa-runtime";
  proof:
    | {
        kind: "no_local_admission";
        run_spec_ref?: DigestBoundRef<"qa.runspec/v1">;
        local_lease_binding_ref?: DigestBoundRef<"qa.local-lease-binding/v1">;
        reason: "source_or_policy_blocked" | "approval_denied" | "dispatch_not_accepted";
      }
    | {
        kind: "prepare_failed_without_resources";
        prepare_result_ref: DigestBoundRef<"qa.prepare-design-result/v1" | "qa.prepare-execution-result/v1">;
      }
    | {
        kind: "empty_inventory";
        resource_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
        resource_inventory_lineage_id: string;
        resource_inventory_version: number;
        resource_inventory_digest: Sha256;
        inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
      };
  completed_at: ISO8601;
  outcome: "not_required";
};

type CleanupReceipt = PerformedCleanupReceipt | NotRequiredCleanupReceipt;
```

Cleanup 必须按 Run ownership tag、ExecutableIdentity/ProcessLaunchBinding/ProcessIdentity、最新 sealed ResourceInventorySnapshot 的 lineage/ref/version/digest、InventorySealReceipt 和 CleanupCapability 操作，禁止按 PID、端口号、进程名或路径模糊匹配其他 Run。Cleanup 幂等身份必须至少包含 `(run_id, environment_id, cleanup_lineage_id, inventory_lineage_id, inventory_version, inventory_digest, capability_sequence, reason, cleanup_attempt)`；同 lineage 的后续 attempt 必须引用前一 CleanupReceipt，禁止用相同 key 覆盖不同 inventory version。`not_found` 只有在 EffectRecord、Warden reconcile 和前次 Receipt 能证明资源已释放时才视为幂等成功，否则必须产生 `resource_missing_without_proof` residual。

闭合规则：

- `succeeded` 要求 inventory 中每项均为 succeeded/not_found，或进入合法 `preserved_resources`，且没有 residual。
- `partially_succeeded` 要求至少一项已处理且至少一个 residual；所有 residual 必须给出 exposure、retryable 和 repair_required。
- `failed` 表示 inventory 无法可信解析、capability 不足或 Cleanup 无法取得任何确定进展，必须进入 `cleanup_repair`。
- `kind="not_required"` 只允许尚未发生 local admission、prepare strict union 明确返回 `failed_without_resources`，或 Runtime 提供由有效 InventorySealReceipt 绑定的 sealed empty inventory snapshot/ref/lineage/version/digest。它禁止伪造 CleanupCapability/inventory reference；proof 必须能独立验证没有本地资源副作用。
- preserved resource 不是 residual，但必须有 retention policy、custodian 和最终删除/解除 hold 的后续责任；活跃 CredentialLease、网络可达进程和浏览器 Profile 禁止 preserve。
- `terminal` 前所有 CredentialLease 必须有 `status="settled"` receipt，所有 materialization 必须有 `kind="released"` Receipt；任何 `credential_active`、`secret_materialized` residual 或 materialization 状态未知都阻止操作完成并触发最高优先级 repair。
- higher-fence cleanup 必须使用 capability successor、`cleanup_takeover` command 和 control-cleanup verifier；它只可减少 residual，禁止恢复 Execution lease、创建新测试资源或修改 sealed predecessor inventory。

### 11.4 ArtifactStore Interface

```ts
type ArtifactStore = {
  put(input: {
    run_id: UUID;
    sanitized_bytes_ref: DigestBoundRef<"qa.sanitized-bytes/v1">;
    redaction_receipt_ref: DigestBoundRef<"qa.redaction-receipt/v1">;
    metadata: Omit<ArtifactPointer, keyof ContractMeta | "artifact_id" | "storage" | "redaction_receipt_ref">;
    idempotency_key: string;
  }): Promise<ArtifactPointer>;
  get(input: {
    pointer: ArtifactPointer;
    actor: ActorRef;
    access_capability: ArtifactAccessCapability;
    range: { offset: number; length: number } | "full";
  }): Promise<AsyncIterable<Uint8Array>>;
  delete(input: {
    pointer: ArtifactPointer;
    retention_policy_ref: DigestBoundRef<"qa.artifact-retention-policy/v1">;
    reason: "retention_expired" | "user_requested" | "run_deleted" | "legal_hold_released" | "repair";
    deletion_idempotency_key: string;
  }): Promise<ArtifactDeletionReceipt>;
};
```

`ArtifactStore.get` 必须同时验证 Pointer digest、actor 与 capability subject、可选 store provider audience、allowed operation/range、TTL、nonce 和 revocation state；object-backed Artifact 的同一 capability 必须同时绑定当前 Runtime 和实际 Store provider，禁止通过第二张隐式 capability 或 adapter exchange 扩权。`RuntimeService.getArtifact` 必须执行相同检查并要求 runtime audience 匹配当前 instance。每次 Store 调用只允许 `full` 或一个连续 range；多个允许 range 必须拆成多次调用，返回 stream 只能包含该次请求区间。Pointer 或 `access_scope.readers` 本身不能替代 capability。`delete` 必须按 `deletion_idempotency_key` 对账，先撤销 access capability，再删除 bytes 与 wrapped DEK，并只返回可持久化的 `ArtifactDeletionReceipt`。

---

## 12. Quality、Publication 与 PQL 契约

### 12.1 FailureClassification

```ts
type FailureClassification =
  | "product_defect"
  | "test_failure"
  | "coverage_gap"
  | "environment_failure"
  | "flaky"
  | "policy_blocked"
  | "insufficient_evidence";
```

### 12.2 QualityEvaluation

```ts
type QualityRuleSet = {
  rule_set_id: string;
  version: string;
  digest: Sha256;
};

type QualityClassification = {
  classification: FailureClassification;
  confidence: number; // 0..1
  case_ids: string[];
  reason_codes: string[];
  evidence_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
};

type ExecutedQualityEvaluation = ContractMeta & {
  kind: "executed";
  evaluation_id: string;
  run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref: DigestBoundRef<"qa.source-acquisition/v1">;
  plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
  case_result_refs: DigestBoundRef<"qa.case-result/v1">[];
  evidence_manifest_refs: DigestBoundRef<"qa.evidence-manifest/v1">[];
  cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
  input_set_digest: Sha256;
  rule_set: QualityRuleSet;
  evaluated_at: ISO8601;
  supersedes_ref?: DigestBoundRef<"qa.quality-evaluation/v1">;
  case_summary: {
    total: number;
    passed: number;
    failed: number;
    error: number;
    skipped: number;
    inconclusive: number;
  };
  coverage: {
    required_scope_digest: Sha256;
    executed_scope_digest: Sha256;
    status: "satisfied" | "partial" | "not_satisfied";
    missing: string[];
  };
  classifications: QualityClassification[];
  execution_outcome: ExecutionOutcome;
  cleanup_outcome: CleanupOutcome;
  evidence_outcome: EvidenceOutcome;
  final_quality_outcome: "pass" | "fail" | "blocked" | "inconclusive";
  publication_eligibility: {
    github: boolean;
    pql: boolean;
    reason_codes: string[];
  };
  dedup_key: string;
  rationale_codes: string[];
};

type NonExecutedQualityEvaluation = ContractMeta & {
  kind: "non_executed";
  evaluation_id: string;
  run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
  run_spec_ref?: DigestBoundRef<"qa.runspec/v1">;
  source_acquisition_ref?: DigestBoundRef<"qa.source-acquisition/v1">;
  policy_decision_ref?: DigestBoundRef<"qa.policy-decision/v1">;
  cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
  reason:
    | "source_blocked"
    | "source_failed"
    | "design_denied"
    | "design_failed"
    | "policy_denied"
    | "execution_denied"
    | "dispatch_failed"
    | "device_unavailable"
    | "prepare_failed_without_resources"
    | "cancelled_before_execution";
  input_set_digest: Sha256;
  rule_set: QualityRuleSet;
  evaluated_at: ISO8601;
  execution_outcome: "blocked" | "cancelled";
  cleanup_outcome: CleanupOutcome;
  evidence_outcome: "not_available";
  final_quality_outcome: "blocked" | "inconclusive";
  publication_eligibility: {
    github: boolean;
    pql: boolean;
    reason_codes: string[];
  };
  dedup_key: string;
  rationale_codes: string[];
};

type QualityEvaluation = ExecutedQualityEvaluation | NonExecutedQualityEvaluation;
```

`quality-evaluation` 必须按 §3.5 严格判别 executed/non-executed variant。规则集和排序后的输入引用必须参与 `input_set_digest`，使同一输入和规则可以重放。Final Quality Outcome 在 Publication 前确定，Publication、Cleanup repair 或 PQL Promotion 的失败禁止原地改写既有 Evaluation；规则变化必须生成带 `supersedes_ref` 的新对象。

只要已经存在任一有效 StepAttemptReceipt、BackendObservation 或 CaseResult，就必须使用 `kind="executed"`。若部分执行后 Amendment、重新批准或 Policy 被拒绝，`execution_outcome` 与 `final_quality_outcome` 可以为 `blocked`，并在 `rationale_codes` 记录 `execution.partial_then_blocked`；禁止退回 non-executed variant 丢失已发生的执行事实。Source/Design/Dispatch/无资源 Prepare 失败必须使用对应明确 reason，禁止挤压成 `source_blocked` 或 `cancelled_before_execution`。

### 12.3 PublicationPlan

```ts
type PublicationAction =
  | {
      type: "github_check";
      repository: string;
      github_head_sha: string;
      tested_effective_sha: string;
      conclusion: "success" | "failure" | "neutral" | "cancelled" | "timed_out" | "action_required";
      title: string;
      summary_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
    }
  | {
      type: "pr_comment";
      repository: string;
      pr_number: number;
      body_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
    }
  | {
      type: "product_issue";
      repository: string;
      title: string;
      body_ref: DigestBoundRef<"qa.artifact-pointer/v1">;
      labels: string[];
      reproduction_digest: Sha256;
    }
  | {
      type: "pql_feedback";
      quality_evaluation_ref: DigestBoundRef<"qa.quality-evaluation/v1">;
    };

type PublicationPlan = ContractMeta & {
  publication_plan_id: string;
  quality_evaluation_ref: DigestBoundRef<"qa.quality-evaluation/v1">;
  publication_policy_ref: DigestBoundRef<"qa.publication-policy/v1">;
  authorized_artifact_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  actions: Array<{
    action_id: string;
    dedup_key: string;
    rendered_content_digest: Sha256;
    action: PublicationAction;
    precondition_codes: string[];
  }>;
};
```

PublicationPlan 中的 body/summary/evidence reference 必须全部来自 `authorized_artifact_refs`，且不得与 `RunSpec.publication_intent` 或 QualityEvaluation eligibility 冲突。GitHub 与 PQL 必须作为独立 Action 存在，不能因一个目标失败阻断另一个目标。

### 12.4 PublicationReceipt

```ts
type PublicationActionReceipt = {
  action_id: string;
  dedup_key: string;
  attempt: number;
  rendered_content_digest: Sha256;
  previous_receipt_ref?: DigestBoundRef<"qa.publication-receipt/v1">;
  outcome: "published" | "updated" | "skipped" | "failed" | "repair_backlog";
  external_ref?: string;
  observed_external_version?: string;
  reconciliation: "created" | "matched_existing" | "updated_existing" | "not_found" | "conflict";
  retryable: boolean;
  retry_budget_exhausted: boolean;
  error?: ErrorEnvelope;
};

type PublicationReceipt = ContractMeta & {
  receipt_id: string;
  publication_plan_ref: DigestBoundRef<"qa.publication-plan/v1">;
  actions: PublicationActionReceipt[];
  outcome: "published" | "partially_published" | "failed" | "skipped" | "settled_with_repair";
  settled_at: ISO8601;
};
```

每个 Action 必须单独达到 published/updated/skipped、不可重试 failed、重试预算耗尽或明确移交 repair backlog 后才算 settled。Repair 只追加新的 Action Receipt，不得重新执行测试或修改 QualityEvaluation。

### 12.5 CoverageGap、Proposal、Review 与 Promotion

```ts
type CoverageGap = ContractMeta & {
  gap_id: string;
  source_run_ref: DigestBoundRef<"qa.runspec/v1">;
  quality_evaluation_ref: DigestBoundRef<"qa.quality-evaluation/v1">;
  project_pack_ref?: DigestBoundRef<"qa.pql-project-pack/v1">;
  gap_type:
    | "missing_case"
    | "missing_scope"
    | "stale_selector"
    | "fixture_gap"
    | "oracle_gap"
    | "evidence_gap"
    | "flaky_pattern";
  affected_scope: string[];
  observed_case_ids: string[];
  evidence_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  severity: "low" | "medium" | "high";
  dedup_key: string;
};

type AssetChangeProposal = ContractMeta & {
  proposal_id: string;
  coverage_gap_ref: DigestBoundRef<"pql.coverage-gap/v1">;
  target_project_pack: {
    id: string;
    base_version: string;
    base_digest: Sha256;
  };
  changes: Array<{
    asset_type: "case" | "fixture" | "selector" | "oracle" | "scope_policy";
    operation: "add" | "update" | "remove";
    asset_id?: string;
    design_only: true;
    proposed_content_ref: DigestBoundRef<"pql.proposed-content/v1">;
    rationale: string;
  }>;
};

type PQLReviewDecision = ContractMeta & {
  review_id: string;
  proposal_ref: DigestBoundRef<"pql.asset-change-proposal/v1">;
  reviewer: ActorRef;
  decision: "approved" | "rejected";
  check_refs: DigestBoundRef[];
  decided_at: ISO8601;
  reason_codes: string[];
};

type ProjectPackPromotionReceiptBase = ContractMeta & {
  promotion_id: string;
  proposal_ref: DigestBoundRef<"pql.asset-change-proposal/v1">;
  review_decision_ref: DigestBoundRef<"pql.review-decision/v1">;
  base_pack_ref: DigestBoundRef<"qa.pql-project-pack/v1">;
};

type ProjectPackPromotionReceipt =
  | (ProjectPackPromotionReceiptBase & {
      outcome: "promoted";
      promoted_pack_ref: DigestBoundRef<"qa.pql-project-pack/v1">;
      promoted_at: ISO8601;
    })
  | (ProjectPackPromotionReceiptBase & {
      outcome: "conflict";
      observed_current_pack_ref: DigestBoundRef<"qa.pql-project-pack/v1">;
      error: ErrorEnvelope;
    })
  | (ProjectPackPromotionReceiptBase & {
      outcome: "failed";
      error: ErrorEnvelope;
    });
```

Proposal 创建后不可原地变为 approved。只有绑定同一 proposal digest 的 approved PQLReviewDecision 和 `outcome="promoted"` 的 ProjectPackPromotionReceipt 才能产生可执行资产。`conflict` 必须记录当前 pack ref 且禁止携带 `promoted_pack_ref`；`failed` 也禁止携带 promoted fields。Promotion 必须校验 base pack digest；新 Project Pack 禁止回灌当前 Run，只能成为后续 Run 的 Source/Design 输入。

### 12.6 Adapter Interfaces

```ts
type QualityEvaluator = {
  evaluateExecuted(input: {
    run_spec_ref: DigestBoundRef<"qa.runspec/v1">;
    plan_ref: DigestBoundRef<"qa.structured-plan/v1">;
    case_result_refs: DigestBoundRef<"qa.case-result/v1">[];
    evidence_manifest_refs: DigestBoundRef<"qa.evidence-manifest/v1">[];
    cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
    rule_set: QualityRuleSet;
  }): Promise<ExecutedQualityEvaluation>;
  evaluateNonExecuted(input: {
    run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
    reason: NonExecutedQualityEvaluation["reason"];
    related_refs: DigestBoundRef[];
    cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
    rule_set: QualityRuleSet;
  }): Promise<NonExecutedQualityEvaluation>;
};

type PublicationAdapter = {
  plan(input: {
    evaluation: QualityEvaluation;
    publication_policy_ref: DigestBoundRef<"qa.publication-policy/v1">;
    authorized_artifact_refs: DigestBoundRef<"qa.artifact-pointer/v1">[];
  }): Promise<PublicationPlan>;
  publish(plan: PublicationPlan): Promise<PublicationReceipt>;
  repair(input: {
    plan: PublicationPlan;
    previous_receipt: PublicationReceipt;
    action_ids: string[];
  }): Promise<PublicationReceipt>;
};

type PQLFeedbackAdapter = {
  deriveGaps(evaluation: QualityEvaluation): Promise<CoverageGap[]>;
  proposeChanges(gap: CoverageGap): Promise<AssetChangeProposal | null>;
  recordReview(decision: PQLReviewDecision): Promise<void>;
  promote(input: {
    proposal: AssetChangeProposal;
    review: PQLReviewDecision;
    idempotency_key: string;
  }): Promise<ProjectPackPromotionReceipt>;
};
```

---

## 13. Workflow 状态机

### 13.1 状态定义

```ts
type WorkflowState =
  | "created"
  | "source_resolving"
  | "design_approval_pending"
  | "designing"
  | "design_cleaning_up"
  | "policy_review"
  | "execution_approval_pending"
  | "dispatching"
  | "preparing"
  | "ready"
  | "executing"
  | "collecting_evidence"
  | "amendment_pending"
  | "amendment_cleaning_up"
  | "amendment_designing"
  | "blocked"
  | "cancelling"
  | "timing_out"
  | "recovering"
  | "cleaning_up"
  | "cleanup_repair_pending"
  | "evaluating"
  | "publishing"
  | "publication_repair_pending"
  | "finalizing"
  | "terminal";
```

### 13.2 主流程

```text
created
→ source_resolving
→ design_approval_pending
→ designing
→ design_cleaning_up
→ policy_review
→ execution_approval_pending
→ dispatching
→ preparing
→ ready
→ executing
→ collecting_evidence
→ cleaning_up
→ evaluating
→ publishing
→ finalizing
→ terminal
```

`WorkflowState` 是 hosted 持久编排状态；`RuntimeRunState` 是设备侧快照。两者必须通过 fenced RuntimeEvent、Checkpoint 和 Receipt 关联，禁止按名称直接映射。

### 13.3 转移表

| 当前状态 | 事件/条件 | 下一状态 | 必须持久化的输出 |
|---|---|---|---|
| `created` | RunDraft 持久化 | `source_resolving` | RunDraft、创建幂等键。 |
| `source_resolving` | SourceAcquisition acquired，RunSpec 冻结 | `design_approval_pending` | SourceAcquisition、SourceRevision、RunSpec。 |
| `source_resolving` | blocked/failed | `blocked` | SourceAcquisition、ErrorEnvelope、reason code。 |
| `design_approval_pending` | approved Evidence 验证成功，strict Design reservation、Design Grant 签发且 DesignCommand 原子 admission 成功 | `designing` | exact authorization preimage、LocalLeaseBinding、Design Grant、CommandAdmissionReceipt、stable design environment id、empty inventory root、signed CleanupCapability、active execution-purpose LocalExecutionLease、FenceTransition、PredecessorFencingRecord、initial Effect/outbox；reservation 在 admission 前不得 fence active generation。 |
| `design_approval_pending` | denied/expired | `blocked` | Evidence 或 expiry reason、审计。 |
| `designing` | Plan 生成或设计失败 | `design_cleaning_up` | StructuredPlan/plan error、DesignEnvironmentReceipt、inventory。 |
| `design_cleaning_up` | Design Cleanup succeeded/not_required | `policy_review` 或 `blocked` | Design CleanupReceipt；失败时保留 design error。 |
| `design_cleaning_up` | Cleanup 有 residual | `cleanup_repair_pending` | CleanupReceipt、repair resume state=`policy_review|blocked`。 |
| `policy_review` | policy deny | `blocked` | PolicyDecision。 |
| `policy_review` | policy allow，需取得 ExecutionApprovalEvidence | `execution_approval_pending` | PolicyDecision、approved envelope、approval request digest。 |
| `execution_approval_pending` | approved Evidence 验证成功，strict Execution reservation 与 exact preimage 已冻结，Execution Grant 签发 | `dispatching` | ExecutionApprovalEvidence、LocalLeaseBinding、Execution Grant、candidate fence；Amendment 上下文同时冻结完整 PlanAmendment 与 ResumeDirective。 |
| `execution_approval_pending` | denied/expired | `blocked` | Evidence 或 expiry reason、审计。 |
| `dispatching` | Runtime 原子接受 Execute/AmendmentResume command 并激活 reservation | `preparing` | CommandAdmissionReceipt、stable execution environment id、sealed signing inputs、empty inventory root、complete CleanupCapability、active LocalExecutionLease、FenceTransition、PredecessorFencingRecord、Grant nonce、initial Effect 与 sequence=1 开始的 RuntimeEvent outbox。 |
| `dispatching` | device offline/Grant expired/dispatch deadline | `blocked` | dispatch reason；禁止静默换设备或本地续期。选择其他设备必须创建带 `parent_run_id` 的新 Run，重新冻结 target device、ApprovalEvidence、LocalLeaseBinding 和 Grant。 |
| `preparing` | VZ Prepare 和 Readiness ready | `ready` | VZSandboxDescriptor/Receipt、PreparedEnvironment、ProcessWardenScope、ReadinessReceipt、versioned inventory、CleanupCapability。 |
| `preparing` | prepare partial failure 或 readiness 失败且已有资源 | `cleaning_up` | PrepareExecutionResult(partial_failure)、ErrorEnvelope、latest inventory snapshot；先完成 effect reconcile 与 inventory seal barrier，再以 CleanupCapability + seal receipt 提交 Cleanup。 |
| `preparing` | prepare failed_without_resources | `evaluating` | PrepareExecutionResult(failed_without_resources)、权威空资源证明。 |
| `ready` | runner 开始 | `executing` | StepAttempt checkpoint。 |
| `executing` | 新动作超 envelope | `amendment_pending` | PlanAmendmentRequest、checkpoint、inventory。 |
| `executing` | 执行结束 | `collecting_evidence` | CaseResult、Observation、StepAttempt Receipt。 |
| `amendment_pending` | quiesce 完成，旧 Grant 撤销且旧 generation fenced | `amendment_cleaning_up` | revocation receipt、old fence、checkpoint。 |
| `amendment_cleaning_up` | Cleanup amendment_pause succeeded | `amendment_designing` | CleanupReceipt、TerminationReceipt、CredentialLeaseReceipt。 |
| `amendment_cleaning_up` | Cleanup 有 blocking residual | `cleanup_repair_pending` | CleanupReceipt、repair resume state=`amendment_designing`。 |
| `amendment_designing` | 新 Design Approval/Grant、Plan vN、Diff 和 Design Cleanup 完成 | `policy_review` | Plan vN、PlanDiff、amendment Design Evidence/Grant、Design CleanupReceipt；PlanAmendment 尚未冻结。 |
| `amendment_pending` 或 `amendment_designing` | 用户/Policy 拒绝 | `cleaning_up` 或 `evaluating` | blocked reason；有资源时先 Cleanup。 |
| `collecting_evidence` | Manifest 完成或失败 | `cleaning_up` | EvidenceManifest 或 evidence error。 |
| 资源持有型非终态 | cancel_requested | `cancelling` | cancellation intent、requested_at、current fence。 |
| `cancelling` | TerminationReceipt settled | `cleaning_up` | TerminationReceipt、inventory。 |
| 资源持有型非终态 | deadline_exceeded | `timing_out` | timeout intent、absolute deadline、current fence。 |
| `timing_out` | 进程域终止已确认或超出强制终止预算 | `cleaning_up` | TerminationReceipt 或 cancellation_unconfirmed error。 |
| 无本地资源的非终态 | cancel_requested/deadline_exceeded | `evaluating` | non-executed reason、not_required CleanupReceipt。 |
| 依赖 Runtime 且副作用状态不确定的状态 | Runtime lost/restart | `recovering` | last cursor、Checkpoint、Snapshot、inventory、new fence request。 |
| `recovering` | signed RecoveryDecision=wait/resume/advance_from_receipt | 原有安全阶段 | snapshot/checkpoint/admission-snapshot/fence/cursor/TTL/nonce-bound RecoveryDecision；仅 resume 可创建 execution-purpose FenceTransition/PredecessorFencingRecord并使用 RecoveryResumeCommand。 |
| `recovering` | signed RecoveryDecision=reconcile_and_seal | `cleaning_up` | `control_quiesce_reconcile` FenceTransition、PredecessorFencingRecord、open inventory；只允许 quiesce/reconcile/terminate/revoke，完成后生成 sealed inventory/InventorySealReceipt并重新请求 cleanup authority。 |
| `recovering` | signed RecoveryDecision=replay_cleanup | `cleaning_up` | `control_cleanup` FenceTransition、PredecessorFencingRecord、successor CleanupCapability、sealed inventory/ref/version/digest、InventorySealReceipt、cleanup_takeover command。 |
| `recovering` | signed RecoveryDecision=irreconcilable | `cleaning_up` 或 `evaluating` | recovery error、RuntimeRepairReceipt；有资源时先 control-only Cleanup，禁止恢复执行。 |
| `cleaning_up` | Cleanup succeeded/not_required | `evaluating` | 全部 CleanupReceipt、settled lease/termination receipt。 |
| `cleaning_up` | Cleanup partial/failed 且需重试或移交 | `cleanup_repair_pending` | CleanupResidual、retry budget、escalation。 |
| `cleanup_repair_pending` | repair succeeded，或失败已不可重试且责任已移交 | 保存的 resume state 或 `evaluating` | Repair Receipt、residual disposition、告警。 |
| `blocked` | 无本地资源或 Cleanup 已 settled | `evaluating` | NonExecutedQualityEvaluation 输入。 |
| `blocked` | 已有本地资源 | `cleaning_up` | inventory、CleanupCapability。 |
| `evaluating` | QualityEvaluation 产生且允许发布 | `publishing` | immutable QualityEvaluation。 |
| `evaluating` | QualityEvaluation 产生且 publication skipped | `finalizing` | QualityEvaluation、skipped PublicationReceipt。 |
| `publishing` | 所有 Action settled | `finalizing` | PublicationReceipt。 |
| `publishing` | 可重试失败或 repair backlog | `publication_repair_pending` | PublicationReceipt、retry budget。 |
| `publication_repair_pending` | Action settled/移交 | `finalizing` | 追加 PublicationReceipt、repair disposition。 |
| `finalizing` | 强制 Receipt、五类 Outcome 和 residual disposition 校验完成 | `terminal` | final snapshot、settled_at。 |

### 13.4 状态机约束与事件优先级

- `terminal` 必须不可逆。terminal 后的 Cleanup/Publication repair 只能创建关联 repair operation 和追加 Receipt，禁止重新进入执行或改写既有 QualityEvaluation。
- 已持久化的 `cancel_requested` 或 `deadline_exceeded` 优先于 Amendment、Step completion 和普通 retry；后到的完成事件只能用于对账。
- 旧 generation、错误 fencing token、错误 FenceTransition purpose 或未 ack 的伪造 cursor 的 command/event/Receipt 即使 sequence 更大，也不得推进 workflow、覆盖 Checkpoint 或启动 Step。新 generation 必须从 sequence=1 的 command_accepted 开始。
- 断线本身不立即产生 `execution_outcome=lost`；只有 Snapshot、inventory 和 effect ledger 无法对账且 RecoveryDecision=irreconcilable 时才能判定 lost。
- Runtime restart 必须先关闭 admission，取得 purpose-specific successor lease/fence，写 FenceTransition/PredecessorFencingRecord并上报 Snapshot；恢复执行必须收到 signed RecoveryDecision 和 RecoveryResumeCommand，恢复清理必须使用 control_cleanup takeover 与 capability successor。禁止仅凭本地 Checkpoint 自动恢复。
- CleanupReceipt 的存在不等于 Cleanup 完成。Cleanup 前必须不存在 pre-barrier unsettled Effect，并持有匹配的 sealed inventory 与 InventorySealReceipt；只有 succeeded/not_required，或 residual 已达到不可重试/预算耗尽且明确移交 repair responsibility，才可继续。
- Amendment 必须创建新的 Design/Execution Grant、新 generation 和新 Sandbox；禁止恢复旧 Sandbox。Source revision 变化禁止走 Amendment，必须创建新 Run。
- Publication 失败不得重跑测试；用户显式 rerun 必须创建新的 `run_id`。

---

## 14. 五类 Outcome、质量结论与失败分类

### 14.1 Outcome 类型

```ts
type ExecutionOutcome = "passed" | "failed" | "cancelled" | "timed_out" | "lost" | "blocked";
type CleanupOutcome = "succeeded" | "partially_succeeded" | "failed" | "not_required";
type EvidenceOutcome = "sufficient" | "partial" | "insufficient" | "not_available";
type PublicationOutcome = "published" | "partially_published" | "failed" | "skipped";
type FinalQualityOutcome = "pass" | "fail" | "blocked" | "inconclusive";

type RunOutcomes = ContractMeta & {
  execution_outcome: ExecutionOutcome;
  cleanup_outcome: CleanupOutcome;
  evidence_outcome: EvidenceOutcome;
  publication_outcome: PublicationOutcome;
  final_quality_outcome: FinalQualityOutcome;
};

type RunSettlement = ContractMeta & {
  outcomes_ref: DigestBoundRef<"qa.run-outcomes/v1">;
  cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
  publication_receipt_refs: DigestBoundRef<"qa.publication-receipt/v1">[];
  residual_refs: ResourceRef[];
  status: "settled" | "settled_with_repair";
  repair_operation_refs: DigestBoundRef<"qa.repair-operation/v1">[];
  settled_at: ISO8601;
};

type RepairOperation = ContractMeta & {
  repair_id: string;
  issuer: "fkst-hosted.authorization-authority" | "fkst-hosted.workflow";
  audience: "fkst-local-qa-runtime" | "fkst-hosted.publication-repair";
  stable_repair_key: string;
  original_run_id: UUID;
  type: "cleanup" | "publication";
  target_refs: DigestBoundRef[];
  target_set_digest: Sha256;
  responsible_owner: ActorRef;
  generation: number;
  fence: ExecutionFence;
  status: "queued" | "running" | "settled";
  attempt_count: number;
  max_attempts: number;
  attempt_receipt_refs: DigestBoundRef<"qa.runtime-repair-receipt/v1" | "qa.publication-receipt/v1">[];
  final_outcome?: "succeeded" | "failed" | "partially_succeeded";
  next_attempt_at?: ISO8601;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  completed_at?: ISO8601;
  signature: SignatureBlock;
};
```

`terminal` 表示 RunSettlement 已持久化，不表示五类 Outcome 全部成功。RepairOperation 是与原 Run 关联的新操作记录，必须以 `stable_repair_key`、责任 owner、独立 generation/fence 和不可变 per-attempt Receipt 表达；禁止覆盖前次 attempt 或只保留最后结果。Repair 禁止改变原 `run_id` 的 terminal snapshot、FinalQualityOutcome 或已完成 Step。

### 14.2 映射原则

- Case 全部通过、覆盖满足、Evidence sufficient，且没有产品质量阻断时，Final Quality 可以为 `pass`。
- assertion failed 且可归类为产品缺陷时，Final Quality 必须为 `fail`。
- Policy 拒绝、Grant 拒绝或用户拒绝执行时，Final Quality 必须为 `blocked`。
- 环境失败、Evidence 不足、Flaky 未决或测试自身失败时，Final Quality 应为 `inconclusive`，除非策略明确要求 fail closed。
- Cleanup failure 必须单独记录并触发运维告警；它不应把已确定的产品测试 pass 自动改成 product fail，但可以阻止 Run 被视为操作完成。
- Publication failure 禁止改变既有 QualityEvaluation，只改变 `publication_outcome`。

### 14.3 Failure Classification 规则

| 分类 | 必要条件 | 默认路由 |
|---|---|---|
| `product_defect` | 固定 revision 可复现、结构化 assertion 失败、Evidence sufficient、排除测试与环境问题 | 被测项目 Issue，可选 PR Check failure。 |
| `test_failure` | Test code、Fixture、Selector 或 Oracle 自身错误 | PQL Issue / Asset Proposal。 |
| `coverage_gap` | 必需用户场景、Scope 或 Case 未执行/不存在 | PQL CoverageGap。 |
| `environment_failure` | Workspace、依赖、App、Middleware、Browser、网络或设备不可用 | Run 记录；重复发生时平台 Issue。 |
| `flaky` | 同一 effective SHA、Plan、输入和环境结果不稳定 | PQL Flaky 记录；不自动建产品 Issue。 |
| `policy_blocked` | Policy 或 Grant 明确禁止动作 | Run 审计；必要时 PlanAmendment。 |
| `insufficient_evidence` | 缺少 required log/trace/screenshot/assertion actual | Run 记录 + PQL Evidence Gap。 |

---

## 15. 错误 Envelope 与错误目录

### 15.1 ErrorEnvelope

```ts
type ErrorPhase =
  | "source_resolution"
  | "identity"
  | "authorization"
  | "dependency"
  | "network"
  | "audit"
  | "design"
  | "policy"
  | "dispatch"
  | "prepare"
  | "readiness"
  | "execute"
  | "evidence"
  | "cleanup"
  | "quality"
  | "publication"
  | "recovery"
  | "update";

type SafeErrorDetails =
  | {
      kind: "contract_validation";
      reason: "unknown_discriminator" | "missing_required_field" | "mixed_variant_fields" | "unknown_field" | "invalid_encoding" | "unsupported_version" | "unsupported_enum";
      field_paths: string[];
      discriminator?: string;
    }
  | {
      kind: "binding_mismatch";
      binding: "content_digest" | "signature" | "device" | "runtime_identity" | "pairing" | "grant" | "policy" | "plan" | "fence" | "inventory" | "checkpoint";
      expected_ref?: DigestBoundRef;
      observed_ref?: DigestBoundRef;
      expected_digest?: Sha256;
      observed_digest?: Sha256;
      mismatched_fields: string[];
    }
  | {
      kind: "sequence_violation";
      stream: "command" | "runtime_event" | "local_ipc_request" | "local_ipc_response" | "revocation_batch" | "grant" | "audit" | "inventory";
      expected_sequence: number;
      observed_sequence: number;
      expected_previous_digest?: Sha256;
      observed_previous_digest?: Sha256;
    }
  | {
      kind: "lifecycle_state";
      object_kind: "reservation" | "lease" | "ipc_session" | "pairing" | "effect" | "inventory" | "cleanup" | "runtime" | "update";
      operation: string;
      observed_state: string;
      allowed_states: string[];
    }
  | {
      kind: "dependency";
      package_manager?: DependencyAcquisitionPolicy["allowed_package_managers"][number];
      package_name?: string;
      package_version?: string;
      registry_host_digest?: Sha256;
      reason: "missing_lockfile" | "unpinned" | "integrity_missing" | "integrity_mismatch" | "registry_denied" | "script_denied" | "budget_exceeded" | "timeout";
    }
  | {
      kind: "network";
      scheme: "https" | "http" | "tcp";
      host_digest: Sha256;
      port: number;
      reason: "destination_not_approved" | "private_or_metadata_address" | "dns_rebinding" | "direct_socket" | "protocol_not_allowed" | "egress_budget_exceeded" | "enforcer_unavailable";
    }
  | {
      kind: "resource_limit";
      limit: keyof ResourceLimitBinding["effective_limits"];
      limit_value: number;
      observed_value: number;
      enforcement_action: "deny" | "throttle" | "terminate_process_domain" | "stop_vm";
    }
  | {
      kind: "storage_integrity";
      component: "sqlite" | "wal" | "audit_chain" | "event_outbox" | "effect_records" | "inventory_heads" | "nonce_sequence_watermarks" | "artifact_store";
      expected_digest?: Sha256;
      observed_digest?: Sha256;
      disposition: "admission_closed" | "quarantined" | "repair_required" | "retry_after_reconcile";
    }
  | {
      kind: "external_status";
      provider: string;
      operation: string;
      status_code?: number;
      provider_error_code?: string;
      retry_after_seconds?: number;
    };

type ErrorFields = {
  error_id: string;
  code: string;
  message: string;
  retryable: boolean;
  phase: ErrorPhase;
  severity: "info" | "warning" | "error" | "critical";
  details?: SafeErrorDetails;
  cause_ref?: ResourceRef;
  step_id?: string;
  attempt?: number;
  occurred_at: ISO8601;
};

type RunErrorEnvelope = ContractMeta & ErrorFields;
type RuntimeErrorEnvelope = RuntimeScopedMeta & ErrorFields;
type ErrorEnvelope = RunErrorEnvelope; // Run-scoped objects 的兼容别名；Runtime-scoped 对象禁止使用
```

`message` 必须可安全展示，不得包含 Secret、token、cookie、Authorization header 或用户绝对路径。敏感诊断必须只通过受限 ArtifactPointer 保存。`SafeErrorDetails` 是 exact-object strict union，JCS 编码后不得超过 4096 bytes；任一数组最多 16 项，任一字符串最多 256 UTF-8 bytes，`field_paths` 只能使用 schema field path，host 只能保存 digest，禁止 URL、query、header value、cookie、argv、environment、文件内容、原始 provider body、用户绝对路径或任意嵌套 map。超过边界或未知 details variant 必须以 `error.safe_details_invalid` 替代为无 details 的安全错误并单独审计，禁止截断后保留可能泄密的前缀。

### 15.2 最小错误目录

| Code | Retryable | 处理要求 |
|---|---:|---|
| `contract.unsupported_version` | 否 | 阻止执行，等待组件升级。 |
| `contract.unsupported_enum` | 否 | 拒绝未知 enum，禁止映射默认值。 |
| `contract.invalid_variant` | 否 | 拒绝未知 discriminator 或缺失 variant required 字段。 |
| `contract.forbidden_field` | 否 | 拒绝混入其他 variant 或安全边界未声明字段。 |
| `contract.digest_mismatch` | 否 | 安全阻断并审计。 |
| `contract.invalid_encoding` | 否 | 拒绝不符合 schema 的 base64/base64url、padding 或二进制编码。 |
| `contract.unsupported_capability_claim` | 否 | 拒绝没有独立信任根或 probe 证据支持的 capability/attestation 声明。 |
| `error.safe_details_invalid` | 否 | 丢弃超界、未知或含敏感字段的 details，返回无 details 的安全错误并审计。 |
| `signature.invalid` | 否 | 签名、key id、purpose domain separation 或签名 payload 任一不匹配时拒绝。 |
| `identity.statement_invalid` | 否 | Runtime identity key、epoch、predecessor、binary binding 或 self-signature 不合法。 |
| `identity.rotation_continuity_failed` | 否 | 禁止把无法证明旧/新 key continuity 的变更当 rotation；要求 reset/re-pair。 |
| `identity.pairing_challenge_invalid` | 否 | challenge request、TTL、nonce、user/device/identity epoch 不匹配。 |
| `identity.pairing_revoked` | 否 | 关闭 pairing-bound admission/read/session，保留最小本地 Cleanup。 |
| `identity.pairing_epoch_stale` | 否 | 拒绝旧 pairing receipt、Grant、attestation、IPC session 或 revocation ack。 |
| `attestation.invalid` | 否 | provider signature、request/challenge/run/purpose/user/device/Runtime binding 或 assurance schema 不合法。 |
| `attestation.revoked` | 否 | 禁止用于新 ApprovalEvidence/Grant；使未消费 pairing-bound authorization 失效。 |
| `canonicalization.invalid_utf8` | 否 | 在 schema/digest/signature 前拒绝非法 UTF-8。 |
| `canonicalization.duplicate_member` | 否 | 解析对象物化前拒绝重复 member name。 |
| `canonicalization.invalid_unicode_scalar` | 否 | 拒绝 lone surrogate 或非 I-JSON Unicode。 |
| `canonicalization.invalid_json_number` | 否 | 拒绝 NaN、Infinity 或非法 number token。 |
| `canonicalization.unsafe_integer` | 否 | 超出安全整数范围时要求 schema 使用十进制字符串。 |
| `source.draft_digest_mismatch` | 否 | 拒绝 Acquisition，保持 RunDraft 不变。 |
| `source.acquisition_failed` | 条件 | 按 SourceAcquisition failed variant 进入 blocked 或重试 resolver。 |
| `source.object_unavailable` | 条件 | 禁止回退到浮动 ref；在 retention 内修复对象。 |
| `source.retention_expired` | 否 | 原 Run 不可再执行；创建新 Run。 |
| `source.merge_conflict` | 否 | blocked，禁止只测 head。 |
| `source.effective_sha_mismatch` | 否 | 终止并 Cleanup。 |
| `approval.invalid_signature` | 否 | 禁止签发 Grant。 |
| `approval.variant_mismatch` | 否 | Design/Execution Evidence 用途不匹配，禁止消费。 |
| `approval.request_digest_mismatch` | 否 | 批准对象与 challenge 不一致，重新请求批准。 |
| `approval.scope_mismatch` | 否 | 禁止签发 Grant。 |
| `grant.invalid_signature` | 否 | Runtime 拒绝命令。 |
| `grant.invalid_variant` | 否 | Design/Execution Grant 字段或 scope 不合法。 |
| `grant.sequence_stale` | 否 | 拒绝旧 sequence 并审计。 |
| `grant.device_mismatch` | 否 | 禁止换绑设备。 |
| `grant.policy_mismatch` | 否 | 禁止执行与 PolicyDecision 不一致的 envelope。 |
| `grant.expired` | 条件 | 返回 hosted 重新授权，禁止本地续期。 |
| `grant.replayed` | 否 | 拒绝并触发安全审计。 |
| `grant.revoked` | 否 | 立即停止未开始动作；CleanupCapability 仍可清理。 |
| `revocation.batch_stale` | 否 | batch 超过 freshness 上限；关闭对应授权路径并请求 fresh delivery/snapshot。 |
| `revocation.sequence_gap` | 是 | 不推进 watermark；请求缺失 batch 或完整 snapshot，禁止猜测。 |
| `revocation.chain_mismatch` | 否 | previous ref/digest 与 durable head 不匹配，拒绝并安全审计。 |
| `revocation.watermark_rollback` | 否 | 拒绝 Grant 或 Artifact fact sequence 倒退。 |
| `artifact.access_revoked` | 否 | 拒绝后续 Artifact read/range，Pointer 与旧 readers 快照不能恢复权限。 |
| `plan.digest_mismatch` | 否 | 阻止执行。 |
| `plan.case_orphan` | 否 | 拒绝未关联 Case 的执行 Step。 |
| `plan.assertion_reference_invalid` | 否 | 拒绝未知或跨 Case assertion。 |
| `plan.evidence_requirement_invalid` | 否 | 拒绝未知、重复或非法 not_required requirement。 |
| `plan.amendment_required` | 是 | 进入 `amendment_pending`，不是普通 Step retry。 |
| `plan.amendment_cleanup_incomplete` | 是 | 阻止新 Sandbox，先 repair 旧环境 residual。 |
| `policy.denied` | 否 | blocked。 |
| `runtime.unhealthy` | 是 | 重试或重新选择设备；设备变化必须重新批准。 |
| `runtime.version_incompatible` | 否 | 请求升级或选择兼容设备。 |
| `runtime.stale_fence` | 否 | 拒绝旧 generation command。 |
| `runtime.command_conflict` | 否 | 相同幂等键不同 request digest；必须在 reservation/fence/cursor/nonce 检查前阻断且不改状态。 |
| `runtime.admission_requirements_unsatisfied` | 条件 | capacity、disk、protocol、schema 或 capability 不满足，禁止 reservation/activation。 |
| `runtime.command_admission_incomplete` | 否 | stable environment、empty inventory、CleanupCapability、lease/fencing/initial effect/outbox 未原子创建，整笔回滚。 |
| `runtime.first_cursor_invalid` | 否 | 新 generation 首个 Event 不是 sequence=1，拒绝 stream/snapshot 并进入 recovery。 |
| `runtime.event_ack_conflict` | 否 | 相同 ack key 不同 event set digest 或 cursor 倒退，禁止推进 acknowledgement。 |
| `runtime.event_generation_stale` | 否 | 事件仅留审计，不推进 workflow。 |
| `runtime.command_deadline_exceeded` | 否 | 进入 timing_out/cleanup，禁止迟到启动。 |
| `runtime.cancellation_unconfirmed` | 是 | 强制 reconcile process domain 并进入 Cleanup repair。 |
| `lease_binding.invalid_signature` | 否 | 拒绝 binding 与后续 Grant admission。 |
| `lease_binding.expired` | 是 | 回收 inert reservation，重新 reserve 并重签 Grant。 |
| `lease_binding.already_consumed` | 否 | 拒绝重复 activation 并审计。 |
| `lease_binding.activation_conflict` | 否 | reservation/Grant/fence/cursor 不一致，active generation 保持不变。 |
| `ledger.integrity_failed` | 否 | Runtime unhealthy，禁止新 reservation，进入人工恢复。 |
| `ledger.integrity_checkpoint_missing` | 否 | 没有覆盖当前 durable high watermark 的 signed checkpoint/passed verification，admission 保持 closed。 |
| `ledger.integrity_root_mismatch` | 否 | SQLite/WAL/row/audit/outbox/effect/inventory/nonce root 任一不匹配，禁止自动修复或清空。 |
| `audit.sequence_gap` | 否 | AuditEvent sequence 不连续或同 sequence 不同 digest，停止 mutating path。 |
| `audit.chain_mismatch` | 否 | previous event/checkpoint ref 或 digest 不匹配，Runtime unhealthy并保留只读诊断。 |
| `audit.checkpoint_rollback` | 否 | 拒绝 sequence/transaction watermark 低于 durable head 的 checkpoint。 |
| `ledger.single_writer_violation` | 否 | 立即停止 mutating path 并安全审计。 |
| `effect_gate.denied` | 否 | 副作用前阻断，按 reason 决定失败或 Amendment。 |
| `effect_gate.reconciliation_required` | 是 | 对账 `dispatching`/`uncertain` effect并进入 `reconciling`，禁止盲目重放。 |
| `effect_gate.inventory_conflict` | 是 | 刷新 snapshot/version 后重新决策，禁止覆盖历史。 |
| `effect_gate.phase_mismatch` | 否 | Design/Execution/cleanup verifier 或 context variant 混用，副作用前拒绝。 |
| `effect_gate.invalid_state_transition` | 否 | 拒绝 canonical EffectState 之外或跳过 reconcile/settled 的转换。 |
| `effect_gate.bootstrap_context_invalid` | 否 | bootstrap context 要求 PlanStep/PreparedEnvironment 或缺 admission/inventory/capability 绑定。 |
| `ipc.binding_invalid` | 否 | LocalIPCBinding 的 identity/pairing/boot/session epoch、peer executable、audience、protocol、TTL 或签名不匹配。 |
| `ipc.session_retired` | 否 | retired session 不接受新 sequence/nonce；调用方必须建立 successor binding。 |
| `ipc.sequence_violation` | 否 | sequence gap/倒退、previous digest 不匹配或相同 sequence 不同 digest；不得进入业务处理。 |
| `ipc.nonce_replayed` | 否 | 除完全相同 transport replay 返回原结果外，拒绝 nonce/authentication id 复用。 |
| `vsock.boot_authentication_failed` | 否 | guest identity、boot、nonce 或 channel binding 不匹配，终止 VM 并 Cleanup。 |
| `process.executable_identity_mismatch` | 否 | 实际 binary/code-signing/image 与 ExecutableIdentity 不符，禁止 launch/materialize。 |
| `process.launch_binding_mismatch` | 否 | argv/cwd/fence/warden/secret 与 ProcessLaunchBinding 不符。 |
| `vz.image_digest_mismatch` | 否 | 禁止启动 VM。 |
| `vz.guest_boot_authentication_failed` | 否 | 终止 VM 并 Cleanup 已登记资源。 |
| `vz.guest_agent_unavailable` | 是 | 先 reconcile VM/process 状态，再决定 retry 或 Cleanup。 |
| `warden.process_identity_mismatch` | 否 | 禁止按复用 PID 终止，进入 reconcile。 |
| `warden.termination_incomplete` | 是 | 记录 remaining resources 并进入 Cleanup repair。 |
| `browser.private_channel_failed` | 是 | 终止专用 Chrome/Profile，禁止暴露或回退到任意 CDP。 |
| `browser.profile_isolation_failed` | 否 | Policy deny，禁止使用个人 Profile。 |
| `browser.capability_unavailable` | 否 | 当前无有效 BrowserEnforcementCapability；reservation 前拒绝 Browser Plan，非 Browser Plan 不受影响。 |
| `browser.proxy_enforcement_failed` | 否 | 无法强制 proxy 或阻断 direct socket，先 deny network，再终止完整 Chrome process domain并隔离 raw output。 |
| `browser.enforcement_lost` | 否 | active Session 的 enforcer identity、配置或 process coverage 失效；禁止 Session 内重试或降级。 |
| `secret_broker.binding_invalid` | 否 | broker executable identity、boot epoch、IPC audience 或 request digest 不匹配，禁止 issue/materialize。 |
| `secret_broker.state_unknown` | 是 | materialize/revoke completion 不确定；形成 blocking residual并禁止等价 lease 重新签发。 |
| `update.signature_invalid` | 否 | 拒绝 staged update 并审计。 |
| `update.anti_rollback_rejected` | 否 | release sequence 低于 watermark、nonce 重放或 selection predecessor 不合法。 |
| `update.staging_receipt_missing` | 否 | 未产生完整 staged receipt，禁止 migration/activation。 |
| `update.activation_journal_invalid` | 否 | activation journal 部分写、digest/predecessor 不匹配或未 fsync；禁止选择 candidate。 |
| `update.activation_order_invalid` | 否 | candidate 在 durable activation intent 前启动，立即停止 candidate 并保持旧 selection。 |
| `update.migration_incomplete` | 否 | migration marker 有 gap、commit 状态不确定或 schema before/after 不匹配，进入 recovery-only。 |
| `update.selection_authority_violation` | 否 | Supervisor/Update Module 尝试直接写 release selection；拒绝并安全审计。 |
| `update.compatibility_failed` | 否 | 不激活不兼容 Runtime/worker/image/schema set。 |
| `update.health_gate_failed` | 是 | 自动 rollback 或保持 unhealthy，拒绝新 Run。 |
| `update.rollback_failed` | 否 | 停止 Runtime mutating path并要求人工修复。 |
| `sandbox.create_failed` | 是 | Cleanup 已登记资源。 |
| `sandbox.scope_violation` | 否 | Local PEP 在副作用前阻断并安全审计。 |
| `dependency.lockfile_required` | 否 | 缺少 frozen lockfile 或 lockfile 在 acquisition 中变化，拒绝依赖执行。 |
| `dependency.integrity_failed` | 否 | archive digest/integrity/provenance 与 lockfile 不匹配，隔离 bytes 并阻断 phase。 |
| `dependency.unpinned` | 否 | 浮动 transitive/git dependency 禁止获取。 |
| `dependency.registry_denied` | 否 | registry identity、redirect 或 path prefix 不在 policy 内。 |
| `dependency.lifecycle_script_denied` | 否 | 未声明或 digest 不符的 lifecycle script 禁止执行。 |
| `resource.hard_ceiling_exceeded` | 否 | 请求超过 RuntimeHardCeilings，reservation/prepare 前拒绝且不可 waiver。 |
| `resource.limit_enforcement_unavailable` | 否 | VZ/Warden/cgroup/rlimit/network/storage 任一 enforcement 未 apply，拒绝启动 untrusted flow。 |
| `resource.limit_violated` | 否 | 生成 violation Receipt 并执行绑定的 deny/throttle/termination。 |
| `network.destination_denied` | 否 | destination/protocol/IP 不在 envelope 或命中 private/link-local/metadata/loopback deny。 |
| `network.flow_receipt_missing` | 否 | 未生成 per-flow Receipt 的连接不得建立或作为成功处理。 |
| `network.egress_budget_exceeded` | 否 | 阻断新流并按 ResourceLimitBinding 终止或收敛。 |
| `credential.lease_expired` | 条件 | 禁止注入，必要时重新审批后签发新 lease。 |
| `credential.process_binding_mismatch` | 否 | CredentialLease 与 ProcessLaunchBinding/ExecutableIdentity/ProcessIdentity 不一致，禁止 materialize。 |
| `credential.revocation_failed` | 是 | 标记 credential_active residual，最高优先级 repair。 |
| `readiness.timeout` | 条件 | 按 Plan retry 后 Cleanup。 |
| `backend.protocol_error` | 条件 | Case error，不得默认通过。 |
| `backend.cancel_timeout` | 是 | 强制终止进程组并要求 TerminationReceipt。 |
| `evidence.redaction_failed` | 条件 | Evidence insufficient，raw object 保持 quarantine 或销毁，禁止创建 Artifact/发布。 |
| `evidence.redaction_policy_invalid` | 否 | 拒绝自然语言-only、unknown rule/action/engine、超界或 redactor identity 不受信的 policy。 |
| `evidence.redaction_output_invalid` | 否 | second-pass、schema、size 或 forbidden-class 检查失败，禁止释放部分 sanitized output。 |
| `evidence.quarantine_exposure_attempt` | 否 | 拒绝通过 RuntimeService/Event/Artifact 暴露 raw quarantine 并安全审计。 |
| `evidence.post_redaction_digest_mismatch` | 否 | Artifact digest 与 RedactionReceipt sanitized digest 不一致，拒绝注册/上传。 |
| `cleanup.capability_invalid` | 否 | 拒绝清理请求并安全审计。 |
| `cleanup.capability_successor_invalid` | 否 | successor 未收窄、sequence/predecessor/proof 不符，拒绝 takeover。 |
| `cleanup.inventory_not_sealed` | 是 | seal barrier 尚有未结算 effect，禁止 Cleanup 并先 reconcile。 |
| `cleanup.inventory_digest_mismatch` | 否 | snapshot ref/lineage/version/digest/seal receipt 任一不匹配时停止清理。 |
| `cleanup.idempotency_lineage_conflict` | 否 | 相同 cleanup key 对应不同 lineage/version/capability sequence，禁止覆盖。 |
| `cleanup.resource_missing_without_proof` | 是 | not_found 无 Effect/Warden/Receipt 证明，形成 residual。 |
| `cleanup.blocking_residual` | 是 | 进入 cleanup_repair_pending，阻止新执行环境。 |
| `cleanup.partial_failure` | 是 | 保存 CleanupResidual 并单独重试。 |
| `quality.inputs_incomplete` | 否 | 禁止生成确定性质量结论。 |
| `quality.rule_unsupported` | 否 | 保留输入，等待支持对应 rule set。 |
| `publication.evaluation_mismatch` | 否 | 拒绝与 QualityEvaluation 不匹配的计划。 |
| `publication.artifact_not_authorized` | 否 | 拒绝发布 allowlist 外 Artifact。 |
| `publication.conflict` | 是 | 通过 dedup key 查询既有对象再 reconcile。 |
| `pql.review_required` | 否 | 未批准 Proposal 禁止 Promotion。 |
| `pql.review_stale_base` | 是 | 重新基于最新 Project Pack 生成 Proposal/Review。 |
| `pql.promotion_digest_mismatch` | 否 | 拒绝发布内容不匹配的 Project Pack。 |
| `pql.promotion_conflict` | 是 | 对账并发 pack 更新，禁止覆盖。 |
| `recovery.sequence_gap` | 是 | 请求 Runtime snapshot。 |
| `recovery.snapshot_fence_mismatch` | 否 | 拒绝用旧 Snapshot 恢复。 |
| `recovery.decision_invalid` | 否 | RecoveryDecision 签名、variant、snapshot/checkpoint/fence/cursor/inventory/TTL/nonce 任一不符。 |
| `recovery.takeover_purpose_mismatch` | 否 | execution 与 control_cleanup FenceTransition 混用，拒绝 Resume/Cleanup。 |
| `recovery.irreconcilable` | 否 | 进入 lost + Cleanup/repair，并要求人工审计。 |

---

## 16. 幂等、重试、并发与恢复

### 16.1 幂等键

所有副作用必须使用以下稳定键或等价结构：

| 副作用 | 幂等键 |
|---|---|
| 创建 RunDraft | `run-draft:<trigger-type>:<repo-locator-digest>:<source-input-digest>:<scope-digest>:<request-key>` |
| SourceAcquisition | `source-acquisition:<run-id>:<run-draft-digest>:<resolver-version>` |
| 冻结 RunSpec | `runspec:<run-id>:<source-acquisition-digest>:<project-policy-digest>` |
| 生成 Plan | `plan:<run-id>:<version>:<design-input-digest>` |
| 签发 Grant | `grant:<run-id>:<type>:<plan-or-design-digest>:<approval-id>:<sequence>` |
| LocalLeaseBinding reservation | `lease-binding:<run-id>:<phase>:<hosted-generation>:<authorization-preimage-digest>:<request-key>` |
| Runtime Command admission | `runtime-command:<run-id>:<generation>:<command-sequence>:<command-type>`，同 key 以完整 request digest 决定 replay/conflict |
| FenceTransition | `fence-transition:<run-id>:<purpose>:<predecessor-generation-or-none>:<successor-generation>` |
| Prepare | `prepare:<run-id>:<generation>:<stable-environment-id>:<plan-or-design-digest>:<device-id>` |
| CredentialLease | `credential-lease:<run-id>:<environment-id>:<secret-ref-digest>:<step-set-digest>:<destination-digest>` |
| Step Attempt | `step:<run-id>:<plan-version>:<generation>:<step-id>:<attempt>` |
| Termination | `terminate:<run-id>:<termination-target-scope-digest>:<reason>` |
| Inventory seal | `inventory-seal:<run-id>:<lineage-id>:<version>:<barrier-sequence>:<inventory-digest>` |
| Redaction | `redaction:<run-id>:<raw-byte-digest>:<redaction-policy-digest>:<redactor-digest>` |
| Artifact | `artifact:<run-id>:<step-id>:<artifact-type>:<post-redaction-byte-digest>` |
| Cleanup | `cleanup:<run-id>:<environment-id-or-none>:<cleanup-lineage-id>:<inventory-lineage-id>:<inventory-version>:<inventory-digest>:<capability-sequence>:<reason>:<attempt>` |
| Quality | `quality:<run-id>:<input-set-digest>:<rule-set-digest>` |
| Publication Action | `publication:<publication-plan-digest>:<action-id>:<rendered-content-digest>` |
| GitHub Check | `github-check:<repo>:<effective-sha>:<quality-dedup-key>` |
| PR Comment | `pr-comment:<repo>:<pr-number>:<quality-dedup-key>` |
| Product Issue | `product-issue:<repo>:<classification>:<reproduction-digest>` |
| PQL Gap | `pql-gap:<project-pack-digest>:<gap-type>:<affected-scope-digest>` |
| PQL Review | `pql-review:<proposal-digest>:<reviewer-id>:<decision>` |
| Project Pack Promotion | `pql-promotion:<proposal-digest>:<review-digest>:<base-pack-digest>` |
| RepairOperation | `repair:<original-run-id>:<type>:<target-set-digest>`；单次 attempt 使用 `repair-attempt:<stable-repair-key>:<generation>:<attempt>` |

幂等键和 canonical request digest 必须存储在副作用记录中。所有 reservation、command admission、Effect、seal、Termination、Cleanup、Artifact、Update 和 Publication 接收方都必须把幂等 lookup 作为第一项状态访问：同 key、同 digest 返回既有不可变结果；同 key、不同 digest 产生 `runtime.command_conflict` 或对应领域冲突，且禁止先读取或改变 mutable reservation/fence/cursor/capacity/nonce、消费 Grant、推进 inventory 或启动副作用。重试必须先读取 Snapshot、effect ledger、inventory seal 和既有 Receipt，再决定 create、update、skip、reconcile 或 repair。

### 16.2 Run Lock、Recovery Ledger 与 Effect Record

```ts
type HostedWorkflowLease = ContractMeta & {
  lease_id: string;
  owner_instance_id: string;
  fence: LeaseFence;
  acquired_at: ISO8601;
  expires_at: ISO8601;
  status: "active" | "released" | "expired" | "superseded";
};

type FencePurpose = "execution" | "control_quiesce_reconcile" | "control_cleanup";

type FenceTransition = ContractMeta & {
  transition_id: string;
  purpose: FencePurpose;
  predecessor_fence?: ExecutionFence;
  successor_fence: ExecutionFence;
  trigger:
    | "initial_admission"
    | "amendment"
    | "cancellation"
    | "timeout"
    | "hosted_owner_takeover"
    | "runtime_restart"
    | "cleanup_recovery"
    | "terminal_repair";
  authorization_ref: DigestBoundRef<"qa.fence-transition-authorization/v1">;
  predecessor_cursor?: RuntimeCursor;
  successor_initial_cursor: RuntimeCursor; // sequence=0 before outbox；first persisted event=1
  transitioned_at: ISO8601;
};

type PredecessorFencingRecord = ContractMeta & {
  record_id: string;
  transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
  takeover_mode: FencePurpose;
  predecessor_local_lease_ref: DigestBoundRef<"qa.local-execution-lease/v1">;
  predecessor_fence: ExecutionFence;
  successor_fence: ExecutionFence;
  predecessor_last_cursor: RuntimeCursor;
  predecessor_effect_set_digest: Sha256;
  predecessor_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  prohibited_predecessor_operations: Array<"command" | "event" | "effect" | "credential_renewal" | "step_resume" | "cleanup_confirmation">;
  fenced_at: ISO8601;
};

type LocalExecutionLeaseBase = ContractMeta & {
  lease_id: string;
  purpose: FencePurpose;
  phase:
    | "design"
    | "amendment_design"
    | "execution"
    | "amendment_execution"
    | "recovery_quiesce"
    | "recovery_cleanup"
    | "terminal_repair";
  activation_authority:
    | { kind: "grant"; binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">; grant_ref: DigestBoundRef<"qa.signed-grant/v1"> }
    | { kind: "cancellation"; cancellation_intent_ref: DigestBoundRef<"qa.cancellation-intent/v1"> }
    | { kind: "timeout"; timeout_intent_ref: DigestBoundRef<"qa.timeout-intent/v1"> }
    | { kind: "recovery"; recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1"> }
    | { kind: "repair"; repair_operation_ref: DigestBoundRef<"qa.repair-operation/v1"> };
  runtime_instance_id: string;
  hosted_workflow: LeaseFence;
  local_execution: LeaseFence;
  activated_by_command_id: string;
  activation_command_ref: DigestBoundRef<"qa.runtime-command/v1">;
  fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
  predecessor: AdmissionPredecessor;
  activated_at: ISO8601;
  last_renewed_at: ISO8601;
  expires_at: ISO8601;
};

type LocalExecutionLease =
  | (LocalExecutionLeaseBase & { status: "active" })
  | (LocalExecutionLeaseBase & {
      status: "released" | "expired" | "superseded";
      ended_at: ISO8601;
      end_reason: "normal" | "cancelled" | "timed_out" | "grant_revoked" | "amendment" | "owner_takeover" | "runtime_restart" | "cleanup_takeover" | "terminal_repair";
      successor_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
    });

type EffectState =
  | "pending"
  | "dispatching"
  | "applied"
  | "denied"
  | "failed_retryable"
  | "failed_final"
  | "uncertain"
  | "reconciling"
  | "suppressed"
  | "settled";

type EffectRecord = ContractMeta & {
  effect_id: string;
  idempotency_key: string;
  request_digest: Sha256;
  request_kind: EffectRequest["kind"];
  parent_command_id: string;
  fence: ExecutionFence;
  state: EffectState;
  state_version: number;
  prior_state?: EffectState;
  attempt: number;
  request_ref: DigestBoundRef<"qa.effect-request/v1">;
  receipt_ref?: DigestBoundRef<"qa.effect-receipt/v1">;
  superseded_by_effect_ref?: DigestBoundRef<"qa.effect-record/v1">;
  external_identity_refs: DigestBoundRef[];
  inventory_before_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  inventory_after_ref?: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  first_persisted_at: ISO8601;
  last_transition_at: ISO8601;
  settled_at?: ISO8601;
};

type RecoveryLedgerSnapshot = ContractMeta & {
  ledger_snapshot_id: string;
  ledger_version: number;
  previous_snapshot_ref?: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  runtime_instance_id: string;
  sqlite_schema_version: number;
  active_hosted_lease_ref?: DigestBoundRef<"qa.hosted-workflow-lease/v1">;
  active_local_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
  pending_local_lease_binding_refs: DigestBoundRef<"qa.local-lease-binding/v1">[];
  accepted_command_refs: DigestBoundRef[];
  command_admission_receipt_refs: DigestBoundRef<"qa.command-admission-receipt/v1">[];
  fence_transition_refs: DigestBoundRef<"qa.fence-transition/v1">[];
  predecessor_fencing_record_refs: DigestBoundRef<"qa.predecessor-fencing-record/v1">[];
  unsettled_effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
  inventory_snapshot_refs: DigestBoundRef<"qa.resource-inventory-snapshot/v1">[];
  inventory_seal_receipt_refs: DigestBoundRef<"qa.inventory-seal-receipt/v1">[];
  credential_lease_refs: DigestBoundRef<"qa.credential-lease/v1">[];
  raw_quarantine_refs: DigestBoundRef<"qa.raw-quarantine-artifact/v1">[];
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  last_event_cursor?: RuntimeCursor;
  hosted_acknowledged_cursor?: RuntimeCursor;
  outbox_high_watermark: number;
  audit_checkpoint_ref: DigestBoundRef<"qa.audit-checkpoint/v1">;
  ledger_integrity_checkpoint_ref: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">;
  ledger_integrity_verification_receipt_ref: DigestBoundRef<"qa.ledger-integrity-verification-receipt/v1">;
  integrity_check: "ok" | "failed";
  captured_at: ISO8601;
};

type RuntimeRepairReceipt = ContractMeta & {
  receipt_id: string;
  authority:
    | {
        kind: "recovery";
        recovery_attempt_id: string;
        recovery_decision_ref: DigestBoundRef<"qa.recovery-decision/v1">;
      }
    | {
        kind: "terminal_repair";
        repair_operation_ref: DigestBoundRef<"qa.repair-operation/v1">;
        stable_repair_key: string;
        repair_attempt: number;
      };
  fence_transition_ref: DigestBoundRef<"qa.fence-transition/v1">;
  predecessor_fencing_record_ref: DigestBoundRef<"qa.predecessor-fencing-record/v1">;
  ledger_snapshot_before_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  runtime_snapshot_before_ref: DigestBoundRef<"qa.runtime-run-snapshot/v1">;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  expected_cursor: RuntimeCursor;
  observed_cursor_before: RuntimeCursor;
  observed_cursor_after: RuntimeCursor;
  reconciled_effects: Array<{
    effect_record_ref: DigestBoundRef<"qa.effect-record/v1">;
    before: EffectState;
    after: EffectState;
    evidence_refs: DigestBoundRef[];
  }>;
  inventory_seal_receipt_refs: DigestBoundRef<"qa.inventory-seal-receipt/v1">[];
  successor_cleanup_capability_refs: DigestBoundRef<"qa.cleanup-capability/v1">[];
  termination_receipt_refs: DigestBoundRef<"qa.termination-receipt/v1">[];
  cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
  credential_lease_receipt_refs: DigestBoundRef<"qa.credential-lease-receipt/v1">[];
  secret_materialization_receipt_refs: DigestBoundRef<"qa.secret-materialization-receipt/v1">[];
  browser_network_receipt_refs: DigestBoundRef<"qa.browser-network-enforcement-receipt/v1">[];
  raw_quarantine_disposition_refs: DigestBoundRef[];
  replayed_outbox_range?: { from_exclusive: RuntimeCursor; through_inclusive: RuntimeCursor; event_set_digest: Sha256 };
  residuals: CleanupResidual[];
  ledger_snapshot_after_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  runtime_snapshot_after_ref: DigestBoundRef<"qa.runtime-run-snapshot/v1">;
  outcome: "succeeded" | "partially_succeeded" | "failed";
  remaining_unknown_effect_ids: string[];
  responsibility_transfer_ref?: DigestBoundRef<"qa.repair-operation/v1">;
  completed_at: ISO8601;
  error?: ErrorEnvelope;
  signature: SignatureBlock;
};
```

`RuntimeRepairReceipt` 必须按 §3.6 以 `purpose="runtime_repair_receipt"` 由 device-bound Runtime key 签名。`authority.kind="recovery"` 必须绑定 RecoveryDecision；`authority.kind="terminal_repair"` 必须绑定有效、未过期且签名正确的 RepairOperation、stable repair key 和 attempt。before/after Snapshot、Checkpoint、FenceTransition、cursor、effect state changes、inventory seals、capability successor、termination/cleanup/credential/browser/quarantine disposition 和 residual responsibility 任一缺失或不一致都不得声明 succeeded。

Canonical EffectState 只允许以下转换：`pending -> dispatching|denied|suppressed`；`dispatching -> applied|failed_retryable|failed_final|uncertain`；`failed_retryable -> pending|suppressed|failed_final`；`uncertain -> reconciling`；`reconciling -> applied|failed_retryable|failed_final|suppressed`；`applied|denied|failed_final|suppressed -> settled`。`settled` 不可逆；`failed_retryable` 禁止在未证明无副作用或完成 reconcile 前回到 pending。每次转换必须 CAS `state_version`，写 outbox event，并保留 prior state；禁止使用旧 `admitted/performing/performed/failed` 状态或把 uncertain 直接当失败重放。

- hosted 必须保证单个 `run_id` 只有一个有效 HostedWorkflowLease；Runtime 必须保证单个 `run_id` 只有一个 active LocalExecutionLease。`purpose="execution"` 可以执行获批 Step；`purpose="control_quiesce_reconcile"` 只能 suppress、quiesce、reconcile、terminate、revoke并产生 seal 输入；`purpose="control_cleanup"` 只能消费已 sealed inventory执行 release/delete/revoke 和写 cleanup/repair Receipt。
- 两种 active lease 必须分别具有单调 generation 和不可预测 fencing token；ExecutionFence 必须同时绑定 hosted 与 local generation。`LocalLeaseBinding.reserved_local_execution` 只保证 candidate generation 大于已分配 generation，不代表 active lease。
- `reserveLocalLeaseBinding` 可以存在多个已过期或未激活历史 reservation，但同一 `(run_id, hosted_workflow generation, idempotency_key, request_digest)` 必须收敛为同一 binding。Reservation、CommandAdmissionReceipt、FenceTransition、PredecessorFencingRecord、Grant nonce 和 lease lifecycle 必须有独立 append-only ledger record，禁止通过覆盖一行隐藏历史。
- execution takeover 必须证明新 Grant/approval/admission，并 fence predecessor 的所有 mutating operations；control-cleanup takeover 必须证明 signed RecoveryDecision/RepairOperation，只允许收窄 cleanup successor capability。两者禁止互相冒充。
- 所有 mutating RuntimeCommand、RuntimeEvent、Checkpoint、Termination/Cleanup/Credential/SecretMaterialization/InventorySeal Receipt 和 StepAttempt EffectRecord 必须携带或引用当前 fence、transition 和 strict `AdmissionPredecessor` lineage；initial generation 明确记录 predecessor absent 且禁止伪造 PredecessorFencingRecord，只有 takeover lineage 必须引用真实 predecessor fencing record。
- 旧 owner 即使恢复网络或拥有更大的 event sequence，也不得写入新 generation、启动 Step、续租 CredentialLease 或确认 Cleanup。
- SQLite ledger 必须由 Rust Runtime 单 writer transaction 更新；command/effect/event outbox、lease activation、inventory version/seal 和 Receipt reference 必须满足 foreign key 与唯一约束。TypeScript worker 崩溃不得破坏 ledger，也不得成为恢复权威。
- Runtime restart 必须执行 SQLite integrity check，建立 local recovery latch、关闭 ordinary admission、使旧 local IPC/vsock session 失效，并扫描 RecoveryLedgerSnapshot、未结算 EffectRecord、active/pending lease、FenceTransition、inventory lineage/seals、CredentialLease/materialization、raw quarantine、Browser proxy receipt 和 outbox。Hosted 决定前只允许只读 discovery、identity validation 和不改变外部资源的 reconcile，禁止创建 `FenceTransition`、successor lease 或破坏性 cleanup。Runtime 上报 Snapshot 后，只有收到与 snapshot/checkpoint/fence/cursor 完整绑定的 signed RecoveryDecision，才能激活 `control_quiesce_reconcile` 或新的 execution admission；inventory seal 后才可激活 `control_cleanup`。禁止自动继续执行。

### 16.3 重试边界

- 可重试 transport error 不得自动重放已开始的非幂等 Step；必须先查询 Runtime Snapshot 和 StepAttempt effect ledger。
- `amendment_required` 禁止作为普通 Backend retry；它必须走 §13 的 hard-revoke 路径。
- cancel/timeout intent 一旦持久化，普通 Step retry、Amendment 和迟到 completion 均不得覆盖。
- Cleanup、Credential revoke、Termination、Artifact upload、Publication Action 和 PQL Promotion 必须可独立重试并有各自 retry budget。
- Cleanup/Publication repair 可以在 terminal 后作为关联 RepairOperation 继续，但不得重跑测试、重新打开原 Run 或改写 QualityEvaluation。
- 用户显式 rerun 必须创建新 `run_id`，并通过 `parent_run_id` 关联；禁止复用旧 Run 清空终态。
- Source object retention 过期、source revision 改变、设备改变或批准对象改变时，禁止仅重签 Grant；必须按相应边界创建新 Run 或重新取得 ApprovalEvidence。

### 16.4 Checkpoint 最小内容

```ts
type RecoveryDecisionBase = ContractMeta & {
  decision_id: string;
  issuer: "fkst-hosted.authorization-authority";
  runtime_snapshot_ref: DigestBoundRef<"qa.runtime-run-snapshot/v1">;
  recovery_ledger_snapshot_ref: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  checkpoint_ref: DigestBoundRef<"qa.workflow-checkpoint/v1">;
  admission_snapshot_ref: DigestBoundRef<"qa.runtime-admission-snapshot/v1">;
  observed_fence: ExecutionFence;
  target_fence: ExecutionFence;
  expected_cursor: RuntimeCursor;
  snapshot_inventory_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
  snapshot_inventory_version: number;
  snapshot_inventory_digest: Sha256;
  issued_at: ISO8601;
  not_before: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RecoveryDecision =
  | (RecoveryDecisionBase & { decision: "wait"; reason_codes: [string, ...string[]]; next_review_at: ISO8601 })
  | (RecoveryDecisionBase & {
      decision: "resume";
      resume_state: Extract<WorkflowState, "dispatching" | "preparing" | "ready" | "executing" | "collecting_evidence">;
      transition_authorization_ref: DigestBoundRef<"qa.fence-transition-authorization/v1">;
      new_local_lease_binding_ref: DigestBoundRef<"qa.local-lease-binding/v1">;
      new_execution_grant_ref: DigestBoundRef<"qa.signed-grant/v1">;
      authorization_input_digest: Sha256;
      admission_requirements_digest: AdmissionRequirementsDigest;
      reusable_effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
      reason_codes: [string, ...string[]];
    })
  | (RecoveryDecisionBase & {
      decision: "reconcile_and_seal";
      transition_authorization_ref: DigestBoundRef<"qa.fence-transition-authorization/v1">;
      target_purpose: "control_quiesce_reconcile";
      allowed_operations: ["quiesce" | "reconcile" | "terminate" | "revoke", ...Array<"quiesce" | "reconcile" | "terminate" | "revoke">];
      reason_codes: [string, ...string[]];
    })
  | (RecoveryDecisionBase & {
      decision: "replay_cleanup";
      transition_authorization_ref: DigestBoundRef<"qa.fence-transition-authorization/v1">;
      target_purpose: "control_cleanup";
      cleanup_reason: CleanupReason;
      predecessor_cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      successor_cleanup_capability_ref: DigestBoundRef<"qa.cleanup-capability/v1">;
      successor_cleanup_capability_digest: Sha256;
      sealed_inventory_snapshot_ref: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      sealed_inventory_version: number;
      sealed_inventory_digest: Sha256;
      inventory_seal_receipt_ref: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
      reason_codes: [string, ...string[]];
    })
  | (RecoveryDecisionBase & {
      decision: "advance_from_receipt";
      receipt_refs: [DigestBoundRef, ...DigestBoundRef[]];
      receipt_set_digest: Sha256;
      resume_state: Extract<WorkflowState, "collecting_evidence" | "cleaning_up" | "evaluating" | "publishing" | "finalizing">;
    })
  | (RecoveryDecisionBase & { decision: "irreconcilable"; reason_codes: [string, ...string[]]; mandatory_cleanup: boolean; repair_operation_ref?: DigestBoundRef<"qa.repair-operation/v1"> });

`RecoveryDecision` 必须按 §3.6 以 `purpose="recovery_decision"` 签名，并在消费前验证 Runtime/RecoveryLedger/Admission Snapshot、Checkpoint、observed/target fence、expected cursor、inventory ref/version/digest、TTL 和一次性 nonce。每个 variant 禁止其他 decision 的字段；`resume` 只允许 execution purpose 且禁止携带 CleanupCapability；`reconcile_and_seal` 只允许 `control_quiesce_reconcile` 和非创建型操作；`replay_cleanup` 必须绑定现有 sealed inventory、seal receipt、`control_cleanup` transition authorization 与等权或更窄 successor capability；`wait` 禁止改变本地 authority；`advance_from_receipt` 只能消费允许推进到目标 state 的完整权威 receipt set。过期、重放、snapshot/checkpoint 已变化、cursor 前进或 target fence 不再为当前 successor 时必须重新签发 Decision，禁止本地修补字段。Runtime startup 在收到 Decision 前不得创建 takeover lease、FenceTransition 或执行破坏性 Cleanup。

type WorkflowCheckpoint = ContractMeta & {
  workflow_state: WorkflowState;
  run_draft_ref: DigestBoundRef<"qa.run-draft/v1">;
  source_acquisition_ref?: DigestBoundRef<"qa.source-acquisition/v1">;
  run_spec_ref?: DigestBoundRef<"qa.runspec/v1">;
  fence?: ExecutionFence;
  cancellation_intent_ref?: DigestBoundRef<"qa.cancellation-intent/v1">;
  timeout_intent_ref?: DigestBoundRef<"qa.timeout-intent/v1">;
  plan_refs: DigestBoundRef<"qa.structured-plan/v1">[];
  active_plan_ref?: DigestBoundRef<"qa.structured-plan/v1">;
  active_grant_ref?: DigestBoundRef<"qa.signed-grant/v1">;
  completed_step_ids: string[];
  step_attempt_receipt_refs: DigestBoundRef<"qa.step-attempt-receipt/v1">[];
  active_step?: { step_id: string; attempt: number };
  runtime_instance_id?: string;
  local_lease_binding_refs: DigestBoundRef<"qa.local-lease-binding/v1">[];
  active_local_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
  fence_transition_refs: DigestBoundRef<"qa.fence-transition/v1">[];
  predecessor_fencing_record_refs: DigestBoundRef<"qa.predecessor-fencing-record/v1">[];
  command_admission_receipt_refs: DigestBoundRef<"qa.command-admission-receipt/v1">[];
  last_runtime_event_cursor?: RuntimeCursor;
  acknowledged_runtime_event_cursor?: RuntimeCursor;
  runtime_snapshot_refs: DigestBoundRef<"qa.runtime-run-snapshot/v1">[];
  recovery_ledger_snapshot_ref?: DigestBoundRef<"qa.recovery-ledger-snapshot/v1">;
  audit_checkpoint_refs: DigestBoundRef<"qa.audit-checkpoint/v1">[];
  ledger_integrity_checkpoint_refs: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">[];
  ledger_integrity_verification_receipt_refs: DigestBoundRef<"qa.ledger-integrity-verification-receipt/v1">[];
  environment_refs: DigestBoundRef<"qa.prepared-environment/v1">[];
  resource_inventory_snapshot_refs: DigestBoundRef<"qa.resource-inventory-snapshot/v1">[];
  inventory_seal_receipt_refs: DigestBoundRef<"qa.inventory-seal-receipt/v1">[];
  effect_record_refs: DigestBoundRef<"qa.effect-record/v1">[];
  sanitized_observation_refs: DigestBoundRef<"qa.sanitized-observation/v1">[];
  cleanup_capability_refs: DigestBoundRef<"qa.cleanup-capability/v1">[];
  credential_lease_refs: DigestBoundRef<"qa.credential-lease/v1">[];
  secret_materialization_receipt_refs: DigestBoundRef<"qa.secret-materialization-receipt/v1">[];
  case_result_refs: DigestBoundRef<"qa.case-result/v1">[];
  evidence_manifest_refs: DigestBoundRef<"qa.evidence-manifest/v1">[];
  termination_receipt_refs: DigestBoundRef<"qa.termination-receipt/v1">[];
  cleanup_receipt_refs: DigestBoundRef<"qa.cleanup-receipt/v1">[];
  quality_evaluation_ref?: DigestBoundRef<"qa.quality-evaluation/v1">;
  publication_receipt_refs: DigestBoundRef<"qa.publication-receipt/v1">[];
  amendment_request_refs: DigestBoundRef<"qa.plan-amendment-request/v1">[];
  amendment_refs: DigestBoundRef<"qa.plan-amendment/v1">[];
  recovery_decision_refs: DigestBoundRef<"qa.recovery-decision/v1">[];
  repair_operation_refs: DigestBoundRef<"qa.repair-operation/v1">[];
};
```

---

## 17. 安全与审计要求

### 17.1 Sandbox

- 必须默认 deny host filesystem，并只显式挂载本 Run 的 immutable source、Workspace 和批准目录；所有 path 必须使用 RootQualifiedPath/Pattern，root identity 在 environment 生命周期内不可替换，symlink、mount、case alias 和 path canonicalization 必须由 Local PEP 逐 segment no-follow 检查。
- 必须以 signed RuntimeHardCeilings、AdmissionRequirements 和 Plan/Policy/Grant envelope 的逐字段最小值限制 CPU、内存、磁盘、进程数、open files、总时长、网络、依赖下载和 quarantine，并把 binding、apply/violation 与实际用量写入 Receipt；hard ceiling 不可由 Run 或 waiver 放宽。
- 必须按 Plan、PolicyDecision、Grant 和 CredentialLease 的交集限制网络；未声明 destination 必须阻断，每次 DNS/redirect/connect 必须生成 NetworkFlowReceipt。
- 包管理器和项目依赖获取必须使用 frozen lockfile、DependencyAcquisitionPolicy/Receipt、批准 registry identity、integrity/provenance 与禁用或 digest-bound lifecycle script；在依赖完整性 Gate 前禁止执行任何下载产物。
- 必须对 App、Middleware、Browser 和 Backend 提供可整体终止的 process domain；每次 spawn 必须绑定 ExecutableIdentity 与 ProcessLaunchBinding，并在 Run 结束、取消、超时或 Grant 撤销后取得覆盖相应 TerminationTargetScope 的 TerminationReceipt。
- 执行任何 PR 代码、Shell、App、Browser 或 Codex Action 的 provider 必须同时具备文件隔离、网络 enforcement、独立进程域和资源限制；缺少任一能力时必须拒绝整个执行 Plan，禁止以“低风险”名义降级。
- Design Sandbox 至少必须做到源码只读、默认禁网、禁生命周期脚本和动态项目配置执行、无长期 Secret；无法保证时必须拒绝 Design Grant。
- 外部 Fork PR 默认必须禁止长期 Secret、写权限 token、生产环境和私网访问。
- 使用 host-side Browser Provider 时必须满足 §10.2 的 required capability，使用每 Run 临时 Profile、forced proxy、direct-socket denial 和 BrowserNetworkEnforcementReceipt，并禁止个人 Profile、Keychain、用户扩展和任意 CDP 暴露。
- guest worker 通道必须是 `BootBoundAuthenticatedVsockSession`，绑定完整 `GuestBootEvidence`、双方 ephemeral key、Runtime/guest boot epoch、generation/fence 和严格 sequence。该机制是 host-verified boot manifest + challenge-response，不宣称硬件 remote attestation；本地控制与 provider/helper 通道必须绑定 LocalIPCBinding 和 peer ExecutableIdentity。Loopback/Unix socket 地址本身不构成信任。
- 所有原始观察与 Artifact bytes 必须先进入 Runtime-only raw quarantine，脱敏成功后只持久化 SanitizedObservation 和 post-redaction digest；raw quarantine 禁止通过 Event、getArtifact、日志或 Publication 暴露。

### 17.2 Secret

- Secret 必须以 opaque `secret_ref` 存在于 Plan 和 Execution Grant；Design Grant 禁止包含 Secret scope，禁止把 Secret 值或本地 lease handle 写入任务正文。
- Secret Broker 必须作为 Warden 管理的独立非特权 helper运行，并根据已验证的 EffectRecord、Execution Grant-derived binding、Step、destination、injection mode、TTL、fence、ProcessDomainDescriptor 与实际 ProcessIdentity 签发 CredentialLease；Broker 不得决定业务授权、扩大 scope 或写 Ledger。
- Local PEP 只能在目标进程启动或代理调用时授权物化。Supervisor 只处理 opaque ref、binding 和签名 Receipt；proxy mode 仅 broker 持有明文，guest injection 与 environment/file mode 必须明确临时 custodian、获准 descendants、继承与擦除范围。任一 binary/argv/cwd/domain/identity 变化都要求新的 launch binding 和 lease authorization。Step 完成、取消、超时、Grant 撤销、Cleanup 或 Runtime recovery 时必须撤销/reconcile lease。
- Secret 值禁止进入 Supervisor、TypeScript Worker、Checkpoint、RuntimeCommand、RuntimeEvent、普通日志、CaseResult、ErrorEnvelope、Artifact、GitHub 内容或 PQL 对象；NyxID Transport 和 Ledger 只能传递或持久化 opaque ref/digest/receipt。
- 允许目的地必须同时满足 Plan envelope、PolicyDecision、Execution Grant、Environment policy 与 CredentialLease，任一更窄限制优先。
- terminal 前所有 CredentialLease 必须有 settled CredentialLeaseReceipt；撤销失败必须形成 `credential_active` CleanupResidual。
- 本地 Runtime 的控制 API 必须认证；POC 的无认证 loopback 服务禁止进入生产。

### 17.3 Grant 与密钥

- Authorization Authority signing key 必须只存在 hosted 受控环境。
- Runtime 必须内置或安全更新 trusted public key set，并支持 key rotation overlap。
- Grant 必须短 TTL、单 Run、单 device、单 Plan、单 sequence、可撤销。
- Runtime 时间偏差超过策略阈值时必须拒绝时间敏感 Grant，并报告 clock skew。

### 17.4 审计事件

```ts
type AuditSubject =
  | { kind: "run"; run_id: UUID; runtime_instance_id?: string }
  | { kind: "runtime"; runtime_instance_id: string; run_absent: true };

type AuditEventBase = {
  schema_version: "qa.audit-event/v1";
  content_digest: Sha256;
  audit_event_id: string;
  audit_sequence: number;
  previous_audit_event_digest?: Sha256;
  subject: AuditSubject;
  actor: ActorRef;
  device_id: string;
  source: "hosted" | "nyxid_adapter" | "runtime" | "launcher" | "guest_agent" | "secret_broker" | "browser_enforcer";
  correlation_id: string;
  occurred_at: ISO8601;
  signature: SignatureBlock;
};

type AuditEvent = AuditEventBase & (
  | {
      event_type: "source_lifecycle";
      action: "run_draft_created" | "source_acquisition_attempted" | "source_acquired" | "source_rejected" | "run_spec_frozen";
      object_refs: [DigestBoundRef, ...DigestBoundRef[]];
      outcome: "accepted" | "rejected" | "failed";
      reason_codes: string[];
    }
  | {
      event_type: "identity_approval";
      action: "attestation_issued" | "attestation_rejected" | "pairing_challenged" | "paired" | "re_paired" | "pairing_revoked" | "approval_challenged" | "approval_decided";
      runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
      runtime_pairing_receipt_ref?: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
      device_attestation_ref?: DigestBoundRef<"qa.device-attestation/v1">;
      approval_evidence_ref?: DigestBoundRef<"qa.approval-evidence/v1">;
      challenge_request_ref?: DigestBoundRef;
      outcome: "accepted" | "rejected" | "revoked";
      reason_codes: string[];
    }
  | {
      event_type: "grant_revocation";
      action: "grant_issued" | "grant_accepted" | "grant_rejected" | "grant_replayed" | "grant_revoked" | "artifact_access_revoked" | "revocation_batch_applied" | "revocation_batch_rejected";
      grant_ref?: DigestBoundRef<"qa.signed-grant/v1">;
      revocation_batch_ref?: DigestBoundRef<"qa.revocation-batch/v1">;
      revocation_delivery_receipt_ref?: DigestBoundRef<"qa.revocation-delivery-receipt/v1">;
      grant_sequence?: number;
      revocation_watermark?: RevocationBatch["watermark"];
      outcome: "accepted" | "rejected" | "revoked";
      reason_codes: string[];
    }
  | {
      event_type: "runtime_control";
      action: "reservation_created" | "reservation_cancelled" | "command_admitted" | "command_rejected" | "lease_activated" | "fence_transitioned" | "event_acknowledged" | "stale_fence_rejected";
      command_ref?: DigestBoundRef<"qa.runtime-command/v1">;
      command_admission_receipt_ref?: DigestBoundRef<"qa.command-admission-receipt/v1">;
      local_lease_ref?: DigestBoundRef<"qa.local-execution-lease/v1">;
      fence_transition_ref?: DigestBoundRef<"qa.fence-transition/v1">;
      control_context:
        | { kind: "admission"; predecessor: AdmissionPredecessor }
        | { kind: "non_admission"; predecessor_absent: true };
      cursor?: RuntimeCursor;
      outcome: "accepted" | "rejected";
      reason_codes: string[];
    }
  | {
      event_type: "effect_policy";
      action: "policy_decided" | "effect_allowed" | "effect_denied" | "effect_dispatched" | "effect_reconciled" | "scope_violation";
      effect_record_ref?: DigestBoundRef<"qa.effect-record/v1">;
      policy_decision_ref?: DigestBoundRef<"qa.policy-decision/v1" | "qa.design-policy-decision/v1">;
      effect_request_digest?: Sha256;
      checked_digests?: CheckedDigests;
      outcome: "allowed" | "denied" | "settled" | "uncertain";
      reason_codes: string[];
    }
  | {
      event_type: "dependency_network_limit";
      action: "dependency_acquired" | "dependency_rejected" | "network_allowed" | "network_denied" | "resource_limit_applied" | "resource_limit_violated";
      dependency_receipt_ref?: DigestBoundRef<"qa.dependency-acquisition-receipt/v1">;
      network_flow_receipt_ref?: DigestBoundRef<"qa.network-flow-receipt/v1">;
      resource_limit_receipt_ref?: DigestBoundRef<"qa.resource-limit-receipt/v1">;
      outcome: "allowed" | "denied" | "violated" | "settled";
      reason_codes: string[];
    }
  | {
      event_type: "credential_process";
      action: "process_launched" | "process_identity_rejected" | "credential_issued" | "credential_materialized" | "credential_released" | "credential_revoked" | "credential_reconciled";
      executable_identity_ref?: DigestBoundRef<"qa.executable-identity/v1">;
      process_launch_binding_ref?: DigestBoundRef<"qa.process-launch-binding/v1">;
      credential_lease_ref?: DigestBoundRef<"qa.credential-lease/v1">;
      receipt_refs: DigestBoundRef[];
      outcome: "accepted" | "rejected" | "settled" | "unknown";
      reason_codes: string[];
    }
  | {
      event_type: "resource_cleanup";
      action: "inventory_updated" | "inventory_sealed" | "termination_completed" | "cleanup_started" | "cleanup_completed" | "cleanup_residual" | "repair_escalated";
      inventory_snapshot_ref?: DigestBoundRef<"qa.resource-inventory-snapshot/v1">;
      inventory_seal_receipt_ref?: DigestBoundRef<"qa.inventory-seal-receipt/v1">;
      cleanup_capability_ref?: DigestBoundRef<"qa.cleanup-capability/v1">;
      receipt_refs: DigestBoundRef[];
      outcome: "settled" | "partially_succeeded" | "failed" | "repair_required";
      reason_codes: string[];
    }
  | {
      event_type: "artifact_redaction";
      action: "quarantine_created" | "redaction_completed" | "redaction_failed" | "artifact_registered" | "artifact_read" | "artifact_deleted" | "quarantine_destroyed";
      quarantine_ref?: DigestBoundRef<"qa.raw-quarantine-artifact/v1">;
      redaction_receipt_ref?: DigestBoundRef<"qa.redaction-receipt/v1">;
      artifact_ref?: DigestBoundRef<"qa.artifact-pointer/v1">;
      outcome: "accepted" | "rejected" | "settled";
      reason_codes: string[];
    }
  | {
      event_type: "recovery_update";
      action: "recovery_decided" | "recovery_reconciled" | "runtime_update_staged" | "runtime_update_activated" | "runtime_update_rolled_back" | "runtime_update_failed";
      recovery_decision_ref?: DigestBoundRef<"qa.recovery-decision/v1">;
      runtime_repair_receipt_ref?: DigestBoundRef<"qa.runtime-repair-receipt/v1">;
      update_manifest_ref?: DigestBoundRef<"qa.runtime-update-manifest/v1">;
      update_receipt_ref?: DigestBoundRef<"qa.runtime-update-receipt/v1">;
      outcome: "accepted" | "rejected" | "settled" | "failed";
      reason_codes: string[];
    }
  | {
      event_type: "quality_publication_pql";
      action: "quality_evaluated" | "publication_planned" | "publication_action_settled" | "coverage_gap_created" | "proposal_reviewed" | "project_pack_promoted";
      object_refs: [DigestBoundRef, ...DigestBoundRef[]];
      outcome: "accepted" | "rejected" | "settled" | "failed";
      reason_codes: string[];
    }
  | {
      event_type: "security_violation";
      action: "signature_rejected" | "digest_rejected" | "replay_rejected" | "ipc_session_rejected" | "ledger_integrity_failed" | "sandbox_escape_blocked" | "quarantine_exposure_blocked";
      safe_error_ref: DigestBoundRef<"qa.error-envelope/v1">;
      affected_object_refs: DigestBoundRef[];
      blocked_before_side_effect: boolean;
      reason_codes: [string, ...string[]];
    }
);

type AuditCheckpoint = RuntimeScopedMeta & {
  checkpoint_id: string;
  issuer: "fkst-local-qa-runtime.audit-authority" | "fkst-hosted.audit-authority";
  runtime_identity_statement_ref?: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  first_audit_sequence: number;
  through_audit_sequence: number;
  first_audit_event_digest: Sha256;
  through_audit_event_digest: Sha256;
  audit_event_set_digest: Sha256;
  previous_checkpoint_ref?: DigestBoundRef<"qa.audit-checkpoint/v1">;
  previous_checkpoint_digest?: Sha256;
  ledger_transaction_high_watermark: number;
  created_at: ISO8601;
  signature: SignatureBlock;
};

type LedgerIntegrityCheckpoint = RuntimeScopedMeta & {
  checkpoint_id: string;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_boot_epoch: string;
  sqlite_schema_version: number;
  ledger_transaction_high_watermark: number;
  wal_checkpoint: { frame_high_watermark: number; database_page_count: number; wal_digest: Sha256 };
  roots: {
    ledger_rows_digest: Sha256;
    audit_checkpoint_ref: DigestBoundRef<"qa.audit-checkpoint/v1">;
    audit_event_set_digest: Sha256;
    event_outbox_digest: Sha256;
    effect_record_set_digest: Sha256;
    inventory_head_set_digest: Sha256;
    nonce_and_sequence_watermark_digest: Sha256;
  };
  previous_integrity_checkpoint_ref?: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">;
  checked_at: ISO8601;
  signature: SignatureBlock;
};

type LedgerIntegrityVerificationReceiptBase = RuntimeScopedMeta & {
  receipt_id: string;
  checkpoint_ref: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">;
  verifier_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  verified_projection: "ledger_integrity/v1";
  verified_at: ISO8601;
  signature: SignatureBlock;
};

type LedgerIntegrityVerificationReceipt =
  | (LedgerIntegrityVerificationReceiptBase & { outcome: "passed"; recomputed_roots_digest: Sha256 })
  | (LedgerIntegrityVerificationReceiptBase & {
      outcome: "failed";
      failed_component: "sqlite" | "wal" | "ledger_rows" | "audit_chain" | "event_outbox" | "effect_records" | "inventory_heads" | "nonce_sequence_watermarks" | "checkpoint_link";
      expected_digest?: Sha256;
      observed_digest?: Sha256;
      error: RuntimeErrorEnvelope;
    });
```

`AuditEvent` 必须按 `audit_sequence` 从 1 开始无间隙 append-only，首项禁止 previous digest，后续项必须精确绑定前一 event digest。每个 variant 只允许本 variant 字段，且 action-specific required refs 固定如下：attestation/pairing/approval action 分别必须携带 attestation、challenge/pairing receipt、ApprovalEvidence 对应 ref；Grant action 必须携带 grant ref/sequence，revocation action 必须携带 batch/receipt/watermark；command admission、lease activation、fence transition 必须使用 `control_context.kind="admission"` 和对应 command/receipt/lease/transition ref，reservation cancel/event ack/stale rejection 使用 `non_admission`；dependency/network/limit、credential/process、resource/cleanup、artifact/redaction、recovery/update action 必须携带与 action 同领域的至少一个 receipt/object ref；security violation 必须携带 safe error ref。出现不相关 ref、缺少 required ref 或用全 optional 空对象表示 action 时必须 `contract.invalid_variant`。Event 使用 producing authority key 对 `purpose="audit_event"` 的 canonical payload 签名；Checkpoint 使用 §3.6 `purpose="audit_checkpoint"`，并按 `audit_event_set/v1` 覆盖连续区间。任何 sequence gap、相同 sequence 不同 digest、checkpoint rollback 或 previous mismatch 都是 integrity failure。

`LedgerIntegrityCheckpoint` 必须由 Rust single writer 在 clean transaction boundary 创建，并用 `ledger_integrity/v1` 同时覆盖 SQLite/WAL、Audit、Event outbox、EffectRecord、inventory heads 和 durable nonce/IPC/revocation sequence watermarks；Checkpoint 与 verification Receipt 必须分别由当前 device-bound Runtime key 和受信 verifier key 按 §3.6 `purpose="ledger_integrity_checkpoint"` / `purpose="ledger_integrity_verification_receipt"` 签名。独立 verifier 必须重算并生成 strict `LedgerIntegrityVerificationReceipt`；passed 才能开放普通 admission。Failed Receipt、缺失前项、无法读取 WAL/row set 或任一 root mismatch 必须使 Runtime unhealthy、关闭 reservation/command、保留只读诊断与最小本地 cleanup recovery，禁止通过新建空 Ledger、跳过坏行或只运行 SQLite pragma 后恢复 healthy。

至少必须记录：

- RunDraft 创建、SourceAcquisition 尝试与 RunSpec 冻结。
- RuntimeIdentityStatement lifecycle、pair/re-pair/revoke/reset、DeviceAttestation、Design/Execution Approval challenge、decision、request ref 和 Evidence ref/digest。
- Design/Execution Grant 签发、传输、接受、拒绝、撤销、sequence 和重放尝试，以及 Grant/Artifact `RevocationBatch` chain、watermark 和 delivery ack。
- HostedWorkflowLease、LocalExecutionLease lifecycle、CommandAdmissionReceipt、FenceTransition、PredecessorFencingRecord、execution/control_cleanup takeover 和 stale-fence rejection。
- phase-specific verifier input、EffectContext、checked digests、canonical EffectState transition、PolicyDecision、Local PEP decision 与每项 scope violation；DependencyAcquisitionReceipt、ResourceLimitBinding/Receipt 和每个 NetworkFlowReceipt。
- RuntimeCommand、RuntimeEvent、state transition、cancel/timeout intent 和 RecoveryDecision。
- ExecutableIdentity、ProcessLaunchBinding、LocalIPCBinding、BootBoundAuthenticatedVsockSession、CredentialLease 签发/注入/续期/撤销/reconcile 和 process-binding failure。
- generalized Termination、inventory seal barrier/receipt、CleanupCapability successor、Cleanup lineage/attempt、Residual、preserved resource 和 repair escalation。
- raw quarantine 创建/销毁、RedactionReceipt、SanitizedObservation、post-redaction Artifact 创建、上传、读取和删除。
- Runtime update manifest/staging/migration、anti-rollback watermark、ReleaseSelection、activation 和 rollback。
- 每个 AuditEvent hash link、AuditCheckpoint、LedgerIntegrityCheckpoint/VerificationReceipt，以及 sequence/root/checkpoint integrity failure。
- QualityEvaluation、PublicationPlan、每个 Action Receipt 和 Publication repair。
- CoverageGap、AssetChangeProposal、PQLReviewDecision 与 ProjectPackPromotionReceipt。

审计事件必须带 actor、device、timestamp、correlation id、content digest 和 strict `AuditSubject`；Run-scoped event 必须带真实 `run_id`，Runtime-scoped event 必须使用 `run_absent=true`，禁止伪造 sentinel Run。审计 payload 和 SafeErrorDetails 禁止记录 Secret 值或其他受限原始诊断。

---

## 18. GitHub 与 PQL 路由规则

### 18.1 GitHub Check

每个 PR Run 应该发布或更新一个稳定 GitHub Check。由于 GitHub Check 必须绑定仓库中可识别的 commit，Check 的 `head_sha` 必须使用 PR `head_sha`；Check 名称、summary、details 和 dedup key 必须明确记录实际测试的 synthetic merge `effective_sha`，并同时显示 base/head SHA。禁止把“Check 绑定 head SHA”误解为只测试了 head SHA。

建议映射：

| Final Quality | GitHub Check conclusion |
|---|---|
| `pass` | `success` |
| `fail` | `failure` |
| `blocked` | `action_required` 或 `neutral`，由 policy 决定 |
| `inconclusive` | `neutral` 或 `action_required` |

### 18.2 PR Comment

PR Comment 可以包含摘要、Case 统计、失败分类和受控 Artifact 链接，但禁止粘贴未脱敏原始日志。重试必须更新同一个 dedup comment 或创建 versioned update，禁止重复刷屏。

### 18.3 产品 Issue

只有同时满足以下条件才可以创建产品 Issue：

1. `classification=product_defect`。
2. 固定 `effective_sha` 可复现。
3. assertion actual 和 expected 已结构化记录。
4. Evidence sufficient。
5. 已排除 environment failure、test failure、flaky 和 policy blocked。
6. PublicationPlan 明确包含 `product_issue`，且策略允许。

### 18.4 PQL

- `test_failure`、`coverage_gap`、`flaky`、`insufficient_evidence` 应由 PublicationPlan 中独立的 `pql_feedback` Action 进入 PQL。
- `stale_selector`、Fixture、Oracle 和 Scope Policy 问题必须生成 CoverageGap 或 AssetChangeProposal，而不是产品 Issue。
- 产品缺陷本身禁止作为 PQL 产品 Issue 复制；PQL 只消费其对测试资产的影响。
- AssetChangeProposal 必须始终 `design_only` 且不可原地修改 review 状态；Review 结果必须是独立、不可变的 PQLReviewDecision。
- 只有 approved ReviewDecision 与成功 ProjectPackPromotionReceipt 才能产生新 Project Pack；base digest 冲突必须停止 Promotion。
- 新 Project Pack 只允许进入后续 Run，禁止回灌当前 Run 或绕过 Plan Amendment。

### 18.5 Publication 顺序

```text
QualityEvaluation
→ PublicationPlan
→ GitHub Check / PR Comment / Product Issue / PQL Feedback
→ PublicationReceipt
```

各 Action 可以并行，但必须独立记录结果。一个 Action 失败不得隐藏其他 Action 的成功，也不得修改 QualityEvaluation。

---

## 19. macOS Runtime Daemon 生命周期

### 19.1 安装与身份

```ts
type RuntimeIdentityStatement = RuntimeScopedMeta & {
  identity_statement_id: string;
  runtime_instance_id: string;
  installation_id: string;
  device_id: string;
  local_user: ActorRef & { type: "user" };
  identity_epoch: number;
  signing_key: {
    algorithm: "ed25519" | "es256";
    key_id: string;
    public_key: Base64UrlNoPad;
    protection: "secure_enclave" | "keychain_non_extractable";
  };
  launcher_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  supervisor_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  previous_identity_statement_ref?: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  rotation_reason?: "scheduled" | "key_compromise" | "runtime_reinstall" | "manual_security_action";
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type RuntimePairingChallenge = RuntimeScopedMeta & {
  challenge_id: string;
  issuer: "fkst-hosted.runtime-pairing-authority";
  pairing_request_ref: DigestBoundRef<"qa.runtime-pairing-request/v1">;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  device_id: string;
  user: ActorRef & { type: "user" };
  expected_identity_epoch: number;
  requested_pairing_epoch: number;
  purpose: "initial_pair" | "re_pair" | "identity_rotation";
  challenge_nonce: string;
  issued_at: ISO8601;
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type RuntimePairingReceiptBase = RuntimeScopedMeta & {
  pairing_receipt_id: string;
  issuer: "fkst-hosted.runtime-pairing-authority";
  challenge_ref: DigestBoundRef<"qa.runtime-pairing-challenge/v1">;
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  device_attestation_ref: DigestBoundRef<"qa.device-attestation/v1">;
  device_id: string;
  user: ActorRef & { type: "user" };
  identity_epoch: number;
  pairing_epoch: number;
  runtime_challenge_signature: SignatureBlock;
  paired_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RuntimePairingReceipt =
  | (RuntimePairingReceiptBase & { status: "active" })
  | (RuntimePairingReceiptBase & {
      status: "revoked";
      revoked_at: ISO8601;
      revocation_reason: "user_unpaired" | "device_removed" | "identity_rotated" | "key_compromise" | "runtime_reset" | "provider_security_action";
      successor_pairing_receipt_ref?: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
    });
```

`RuntimeIdentityStatement` 必须由该 statement 声明的 device-bound Runtime key 按 §3.6 `purpose="runtime_identity_statement"` 自签，并由安装时的 signed launcher 验证 binary identity、Keychain/Secure Enclave key handle 与 `runtime_instance_id` 的本地绑定。首次 identity 使用 `identity_epoch=1` 且禁止 previous/rotation 字段；rotation 必须保持 `runtime_instance_id`、递增 identity epoch、绑定前一 statement，并由旧 key 与新 key 分别证明 continuity。旧 key compromise 无法提供 continuity 时不得伪造 rotation，必须走 reset。

`RuntimePairingChallenge` 由 hosted pairing authority 按 §3.6 `purpose="runtime_pairing_challenge"` 签名且 TTL 必须短；Runtime 必须用当前 identity key 对完整 challenge 签名，DeviceAttestation 必须使用 `purpose="runtime_pairing"` 并绑定同一 challenge/request/user/device/identity epoch。`RuntimePairingReceipt` 的外层签名使用 §3.6 `purpose="runtime_pairing_receipt"`；只有 `status="active"`、未过期且 identity/pairing epoch 与当前状态相等的 Receipt 才可用于 Grant、LocalIPCBinding 或 revocation delivery。

Identity key rotation 必须递增 `identity_epoch` 并强制 re-pair；普通 re-pair 保持 identity epoch、严格递增 `pairing_epoch`，同时 retire 旧 LocalIPC session、旧 pending reservation 和尚未消费的旧 pairing-bound Grant。Pairing revocation 必须立即关闭 reservation、command、artifact read 和 transport-control ack path，并使绑定该 pairing epoch 的 DeviceAttestation、Grant 与 IPC session 无效；Cleanup 只能通过已经持久化且不依赖旧 pairing 在线有效性的本地 CleanupCapability 收敛。Runtime reset 必须销毁旧私钥与 endpoint/session material、撤销旧 pairing、分配新的 `runtime_instance_id` 和 `installation_id`，以 identity epoch=1、pairing epoch=0 重新开始；禁止把 reset 伪装成 rotation 或沿用旧 Grant、Ledger authority、nonce watermark、IPC sequence 或 attestation。

Local QA Runtime v1 必须：

- 作为签名、notarized 的 macOS 安装包或等价可信分发物发布。
- 安装固定 bundle identifier 和可验证 Team ID。
- 只注册当前登录用户上下文中的 user LaunchAgent，并由其启动版本稳定的签名 launcher；launcher 根据签名 RuntimeReleaseSelection 启动选定 Rust Supervisor。v1 禁止安装 root/system LaunchDaemon、常驻特权 helper、由可更新 Runtime 自行选择任意 release，或引入第二本地授权权威。
- 建立唯一 `runtime_instance_id`、device-bound non-exportable signing key、`RuntimeIdentityStatement` 和独立 pairing epoch，但禁止复用 NyxID Node identity 作为 Runtime identity。
- 将本地持久状态放在受权限控制的 application support 目录；禁止写入被测仓库。

### 19.2 launchd

`launchd` 配置必须：

- 在用户登录后启动 Runtime；没有活跃用户会话时不得启动需要宿主 Chrome 的 Run。
- 配置 crash restart/backoff，避免无限快速重启。
- 禁止监听公网地址；Runtime API 只监听 loopback 或受控 Unix domain socket。
- 将 stdout/stderr 导向受控日志，并执行大小和保留期限制。
- 每次启动必须扫描 local Ledger、未结算 ResourceInventory、CredentialLease 和 CleanupCapability，建立 local recovery latch并失效旧 IPC/vsock session；Hosted Decision 前只做只读 discovery并上报 Snapshot，不得创建 FenceTransition、successor lease/capability或恢复 Cleanup。收到 signed RecoveryDecision 后才按 `control_quiesce_reconcile`/`control_cleanup` purpose收敛，且在接受新 Run 前解除 recovery blocking state。
- 在升级期间支持 drain：停止接收新 Run，允许安全 checkpoint 或取消当前 Run。

### 19.3 本地认证

- Node/Adapter、本地 CLI、BrowserProvider 和受控 helper 调用 Runtime 时必须使用签名 `LocalIPCBinding`，并通过 Unix peer credential 或 loopback mTLS 绑定实际调用方 ExecutableIdentity；仅持有 endpoint、端口或 bearer token 不足以认证。
- LocalIPCBinding 必须绑定 RuntimeIdentityStatement、active RuntimePairingReceipt、identity/pairing/boot/session epoch、server/client executable identity、service audience、protocol version、peer credential policy、direction、TTL 和 nonce，按 §3.6 `purpose="local_ipc_binding"` 验签；retired session、sequence/digest chain gap、nonce 重放、过期、binary 替换或 peer identity 不匹配必须拒绝。
- `probeHealth(detail="public")` 可以返回非敏感健康信息，但 authenticated health、Run、event ack 和 Artifact 必须认证。
- 未认证请求不得 reservation、提交/取消/清理命令、查询详细 Run、ack event 或下载 Artifact；`getArtifact` 还必须拒绝 raw quarantine ref。

### 19.4 健康与能力报告

```ts
type RuntimeOperation = "probe" | "reserve_non_browser" | "reserve_browser" | "activate_execution" | "execute_non_browser" | "execute_browser" | "quiesce_reconcile" | "cleanup" | "artifact_read" | "event_delivery" | "revocation_delivery" | "redaction" | "update_stage" | "update_activate";

type RuntimeDegradedOperationMatrix = RuntimeScopedMeta & {
  matrix_id: string;
  dimensions: Array<"identity" | "pairing" | "revocation" | "ledger" | "audit" | "disk" | "outbox" | "guest" | "hard_limits" | "network" | "redaction" | "browser_enforcement" | "secret_broker" | "recovery" | "update">;
  rules: Array<{
    condition_code: string;
    allowed_operations: RuntimeOperation[];
    denied_operations: RuntimeOperation[];
    required_reason_codes: string[];
  }>;
  security_operations_without_waiver: Array<"execute_browser" | "cleanup" | "quiesce_reconcile">;
  signature: SignatureBlock;
};

type RuntimeHealth = RuntimeScopedMeta & {
  runtime_identity_statement_ref: DigestBoundRef<"qa.runtime-identity-statement/v1">;
  runtime_pairing_receipt_ref?: DigestBoundRef<"qa.runtime-pairing-receipt/v1">;
  runtime_identity_epoch: number;
  runtime_pairing_epoch: number;
  runtime_boot_epoch: string;
  local_ipc_session_epoch: number;
  status: "healthy" | "degraded" | "draining" | "recovering" | "unhealthy";
  admission: {
    state: "open" | "closed";
    reason_codes: string[];
    accepting_reservations: boolean;
    accepting_commands: boolean;
  };
  recovery: {
    required: boolean;
    active_recovery_attempt_ids: string[];
    unresolved_effect_count: number;
    blocking_residual_count: number;
  };
  runtime_version: string;
  launcher_version: string;
  active_release_sequence: number;
  release_selection_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
  protocol_versions: string[];
  platform: {
    os: "macos";
    version: string;
    arch: "arm64" | "x86_64";
  };
  capabilities: string[];
  capability_digest: Sha256;
  runtime_hard_ceilings_ref: DigestBoundRef<"qa.runtime-hard-ceilings/v1">;
  runtime_hard_ceilings_digest: Sha256;
  browser_enforcement_capability_ref?: DigestBoundRef<"qa.browser-enforcement-capability/v1">;
  secret_broker_binding_ref?: DigestBoundRef<"qa.secret-broker-binding/v1">;
  capacity: {
    max_active_runs: number;
    active_runs: number;
    reserved_runs: number;
    available_vm_slots: number;
    available_browser_slots: number;
    available_port_slots: number;
    available_cpu_millis: number;
    available_memory_bytes: number;
    available_process_count: number;
    available_open_file_count: number;
    available_host_storage_bytes: number;
  };
  disk: {
    volume_id: string;
    total_bytes: number;
    free_bytes: number;
    required_reserve_bytes: number;
    pressure: "normal" | "warning" | "critical";
  };
  ledger: {
    integrity: "ok" | "checking" | "failed";
    integrity_checkpoint_ref?: DigestBoundRef<"qa.ledger-integrity-checkpoint/v1">;
    verification_receipt_ref?: DigestBoundRef<"qa.ledger-integrity-verification-receipt/v1">;
    audit_checkpoint_ref?: DigestBoundRef<"qa.audit-checkpoint/v1">;
    wal_bytes: number;
    emergency_headroom_bytes: number;
  };
  outbox: { pending_events: number; pending_bytes: number; pressure: "normal" | "warning" | "critical" };
  sandbox_provider: "apple_virtualization_framework_linux_vm";
  typescript_worker_bundle_version: string;
  guest_image_digest: Sha256;
  sqlite_schema_version: number;
  sqlite_reader_schema_range: { min: number; max: number };
  sqlite_writer_schema_range: { min: number; max: number };
  migration_state: "none" | "preparing" | "committed" | "rollback_required" | "blocked";
  active_activation_request_ref?: DigestBoundRef<"qa.runtime-activation-request/v1">;
  degraded_operation_matrix_ref: DigestBoundRef<"qa.runtime-degraded-operation-matrix/v1">;
  last_self_check_at: ISO8601;
};
```

Runtime 必须在接受 reservation 和 Grant 前声明兼容 protocol/schema/capabilities，并满足 `AdmissionRequirements` 的所有逻辑配额和 emergency headroom。`status="recovering"|"draining"|"unhealthy"`、identity/pairing 无效、revocation freshness/chain 未收敛、hard ceilings 过期、Audit/Ledger integrity 未完成、disk/ledger/outbox critical 或 recovery blocking residual 存在时 `admission.state` 必须为 closed。Browser enforcement capability 或 Secret Broker binding 缺失时只关闭对应操作，不得伪装为全局 healthy；具体允许操作由签名/摘要绑定的 `RuntimeDegradedOperationMatrix` 决定。hosted 必须在 device selection、reservation 和 dispatch 前检查 compatibility 与 snapshot expiry；Runtime 必须在 admission transaction 中重新检查，禁止依赖陈旧 health response。

### 19.5 升级与回滚

```ts
type RuntimeReleaseSelection = RuntimeScopedMeta & {
  selection_id: string;
  issuer: "fkst-runtime-release-authority";
  launcher_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  release_manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
  release_sequence: number;
  selected_runtime_version: string;
  compatibility_set_digest: Sha256;
  rust_bundle_digest: Sha256;
  typescript_worker_bundle_digest: Sha256;
  guest_image_digest: Sha256;
  sqlite_schema_version: number;
  previous_selection_ref?: DigestBoundRef<"qa.runtime-release-selection/v1">;
  selected_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RuntimeUpdatePolicy = RuntimeScopedMeta & {
  policy_id: string;
  activation_deadline_seconds: number;
  health_confirmation_seconds: number;
  require_zero_active_runs: true;
  allow_automatic_rollback: boolean;
  allow_reverse_migration: boolean;
  minimum_emergency_disk_headroom_bytes: number;
  signature: SignatureBlock;
};

type UpdateStagingInventory = RuntimeScopedMeta & {
  staging_inventory_id: string;
  manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
  release_sequence: number;
  staged_object_refs: [DigestBoundRef, ...DigestBoundRef[]];
  staged_object_set_digest: Sha256;
  staging_root_token: string;
  verified_team_id: string;
  verified_at: ISO8601;
};

type RuntimeActivationRequest = RuntimeScopedMeta & {
  activation_request_id: string;
  manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
  staged_receipt_ref: DigestBoundRef<"qa.runtime-update-receipt/v1">;
  staging_inventory_ref: DigestBoundRef<"qa.update-staging-inventory/v1">;
  predecessor_selection_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
  requested_selection: RuntimeReleaseSelection;
  schema_before: number;
  required_reader_schema_range: { min: number; max: number };
  required_writer_schema_range: { min: number; max: number };
  activation_deadline_at: ISO8601;
  activation_nonce: string;
  signature: SignatureBlock;
};

type LauncherActivationJournalEntry = RuntimeScopedMeta & {
  journal_entry_id: string;
  activation_request_ref: DigestBoundRef<"qa.runtime-activation-request/v1">;
  attempt: number;
  state:
    | "activation_intent_durable"
    | "candidate_started"
    | "migration_committed"
    | "health_evidence_durable"
    | "selection_committed"
    | "rolled_back"
    | "failed";
  previous_entry_ref?: DigestBoundRef<"qa.launcher-activation-journal-entry/v1">;
  schema_before: number;
  schema_after?: number;
  selection_before_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
  selection_after_ref?: DigestBoundRef<"qa.runtime-release-selection/v1">;
  recorded_at: ISO8601;
  signature: SignatureBlock;
};

type RuntimeActivationHealthEvidence = RuntimeScopedMeta & {
  evidence_id: string;
  activation_request_ref: DigestBoundRef<"qa.runtime-activation-request/v1">;
  candidate_executable_identity_ref: DigestBoundRef<"qa.executable-identity/v1">;
  protocol_versions: string[];
  schema_reader_range: { min: number; max: number };
  schema_writer_range: { min: number; max: number };
  sqlite_integrity: "passed";
  ledger_self_check: "passed";
  effect_gate_self_check: "passed";
  guest_capability_self_check: "passed";
  browser_capability_advertised: boolean;
  checked_at: ISO8601;
  signature: SignatureBlock;
};

type RuntimeActivationResult = RuntimeScopedMeta & {
  result_id: string;
  activation_request_ref: DigestBoundRef<"qa.runtime-activation-request/v1">;
  final_journal_entry_ref: DigestBoundRef<"qa.launcher-activation-journal-entry/v1">;
  outcome: "activated" | "rolled_back" | "failed";
  selected_release_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
  health_evidence_ref?: DigestBoundRef<"qa.runtime-activation-health-evidence/v1">;
  error?: RuntimeErrorEnvelope;
  completed_at: ISO8601;
  signature: SignatureBlock;
};

type StableRuntimeLauncher = {
  probeSelectedRelease(): Promise<RuntimeReleaseSelection>;
  activate(request: RuntimeActivationRequest): Promise<RuntimeActivationResult>;
  reconcileActivation(requestRef: DigestBoundRef<"qa.runtime-activation-request/v1">): Promise<RuntimeActivationResult>;
};

type RuntimeUpdateManifest = RuntimeScopedMeta & {
  update_id: string;
  issuer: "fkst-runtime-release-authority";
  release_sequence: number;
  minimum_allowed_release_sequence: number;
  supersedes_manifest_ref?: DigestBoundRef<"qa.runtime-update-manifest/v1">;
  from_version_range: string;
  target_runtime_version: string;
  source_protocol_version_range: string;
  target_protocol_versions: string[];
  source_sqlite_schema_range: { min: number; max: number };
  target_sqlite_schema_version: number;
  migration: {
    plan_digest: Sha256;
    forward_migration_digest: Sha256;
    reverse_migration_digest?: Sha256;
    candidate_reader_schema_range: { min: number; max: number };
    candidate_writer_schema_range: { min: number; max: number };
    previous_reader_schema_range: { min: number; max: number };
    previous_writer_schema_range: { min: number; max: number };
    migration_strategy: "expand_contract" | "forward_only" | "signed_reverse_migration";
    checkpoint_compatibility_versions: string[];
    recovery_ledger_compatibility_versions: string[];
  };
  compatibility_set_digest: Sha256;
  rust_bundle_digest: Sha256;
  typescript_worker_bundle_digest: Sha256;
  guest_image_ref: DigestBoundRef<"qa.vz-linux-image/v1">;
  installer_package_signature: SignatureBlock;
  team_id: string;
  rollout_policy_ref: DigestBoundRef<"qa.runtime-update-policy/v1">;
  issued_at: ISO8601;
  expires_at: ISO8601;
  nonce: string;
  signature: SignatureBlock;
};

type RuntimeUpdateReceipt =
  | (RuntimeScopedMeta & {
      kind: "staged";
      receipt_id: string;
      manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
      release_sequence: number;
      compatibility_set_digest: Sha256;
      staged_rust_bundle_digest: Sha256;
      staged_worker_bundle_digest: Sha256;
      staged_guest_image_digest: Sha256;
      migration_plan_digest: Sha256;
      staging_inventory_ref: DigestBoundRef<"qa.update-staging-inventory/v1">;
      staged_at: ISO8601;
      signature: SignatureBlock;
    })
  | (RuntimeScopedMeta & {
      kind: "activated";
      receipt_id: string;
      manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
      staged_receipt_ref: DigestBoundRef<"qa.runtime-update-receipt/v1">;
      activation_request_ref: DigestBoundRef<"qa.runtime-activation-request/v1">;
      activation_result_ref: DigestBoundRef<"qa.runtime-activation-result/v1">;
      final_journal_entry_ref: DigestBoundRef<"qa.launcher-activation-journal-entry/v1">;
      health_evidence_ref: DigestBoundRef<"qa.runtime-activation-health-evidence/v1">;
      release_selection_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
      previous_version: string;
      active_version: string;
      release_sequence: number;
      sqlite_schema_version: number;
      migration_result_digest: Sha256;
      health_check_digest: Sha256;
      activated_at: ISO8601;
      signature: SignatureBlock;
    })
  | (RuntimeScopedMeta & {
      kind: "rolled_back";
      receipt_id: string;
      manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
      staged_receipt_ref: DigestBoundRef<"qa.runtime-update-receipt/v1">;
      activation_request_ref: DigestBoundRef<"qa.runtime-activation-request/v1">;
      activation_result_ref: DigestBoundRef<"qa.runtime-activation-result/v1">;
      final_journal_entry_ref: DigestBoundRef<"qa.launcher-activation-journal-entry/v1">;
      attempted_selection_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
      restored_selection_ref: DigestBoundRef<"qa.runtime-release-selection/v1">;
      attempted_version: string;
      restored_version: string;
      restored_release_sequence: number;
      schema_rollback_digest?: Sha256;
      reason_codes: string[];
      rolled_back_at: ISO8601;
      signature: SignatureBlock;
    })
  | (RuntimeScopedMeta & {
      kind: "failed";
      receipt_id: string;
      manifest_ref: DigestBoundRef<"qa.runtime-update-manifest/v1">;
      staged_receipt_ref?: DigestBoundRef<"qa.runtime-update-receipt/v1">;
      activation_request_ref?: DigestBoundRef<"qa.runtime-activation-request/v1">;
      activation_result_ref?: DigestBoundRef<"qa.runtime-activation-result/v1">;
      final_journal_entry_ref?: DigestBoundRef<"qa.launcher-activation-journal-entry/v1">;
      stage: "verify" | "drain" | "stage" | "migrate" | "select_release" | "activate" | "health_check" | "rollback";
      release_sequence: number;
      error: RuntimeErrorEnvelope;
      failed_at: ISO8601;
      signature: SignatureBlock;
    });
```

- `RuntimeUpdateManifest.signature` 必须按 §3.6 的 `purpose="runtime_update_manifest"` 验证；installer package signature 必须独立验证 Team ID。Manifest 必须绑定 release sequence、minimum allowed sequence、TTL/nonce、compatibility set、bundle/image digest、protocol/schema ranges、forward/reverse migration digest、checkpoint/recovery-ledger compatibility 和 rollout policy。任何 release sequence 低于已持久化 anti-rollback watermark、重复 nonce、过期 manifest 或兼容集合部分不匹配都必须拒绝。
- Runtime 有 active Run 时禁止直接替换二进制；必须 drain、checkpoint 并完成或取消 Run。
- 升级失败必须自动回滚到上一已知可用版本，或进入 unhealthy 并拒绝新 Run。
- 回滚不得重用新版本已经接受但旧版本无法理解的 checkpoint；hosted 应选择兼容 Runtime 或人工介入。
- Runtime 必须保留一个版本稳定、最小职责的 signed launcher。Launcher 只信任 `fkst-runtime-release-authority`，按 §3.6 `purpose="runtime_release_selection"` 验证 `RuntimeReleaseSelection`，并以单调 `release_sequence` 选择 A/B 或等价原子目录；新 Rust binary、TypeScript worker bundle 和 guest image 必须作为一个 compatibility set 激活，禁止部分替换或由待升级 Runtime 自行改写 launcher authority。
- 下载和验证 compatibility set 后必须先产生签名 `RuntimeUpdateReceipt(kind="staged")`；没有 staged receipt 禁止 migration 或 activation request。Update Module 只能 stage、drain、preflight并生成 signed `RuntimeActivationRequest`，不能直接写 selection。Stable Launcher 必须先把 activation intent fsync 到独立 `LauncherActivationJournalEntry`，再启动 candidate；下一可用 Supervisor 按 journal digest 幂等镜像 `RuntimeUpdateReceipt`，Launcher 不直接写 Run Ledger。
- 数据库 migration 必须严格匹配 manifest 的 source schema range、migration plan digest 和 reader/writer compatibility。v1 自动 rollback 只有在 previous release 对迁移后 schema 同时具备 reader 与 writer compatibility 时允许；若选择 signed reverse migration，必须完整执行、验签并生成 journal evidence。仅“可读”不足以允许旧 writer 重新上线。无法安全回滚时必须保持 candidate/previous stopped 或 recovery-only、admission closed并请求人工修复，禁止用旧 binary 猜测 Ledger。
- activation 状态固定为 `activation_intent_durable → candidate_started → migration_committed → health_evidence_durable → selection_committed`，任一步失败进入 `rolled_back|failed`；每个 journal entry 都绑定前项、schema before/after 和 selection before/after。激活后必须先完成本地认证、SQLite integrity、Ledger/EffectGate self-check、VZ capability、Process Warden，以及条件性的 Browser enforcement capability probe，再接受新 reservation。Browser probe 失败只移除 Browser capability；核心安全自检失败必须 rollback 或 unhealthy。
- 版本报告、anti-rollback watermark、ReleaseSelection、ActivationRequest/Result、HealthEvidence 和每次 staged/activated/rolled_back/failed Receipt 必须进入审计；Runtime-scoped update failure 必须使用 `RuntimeErrorEnvelope(phase="update")`。

### 19.6 卸载

卸载必须：

- 停止并移除 launchd job。
- 撤销本地 credential 和 device/runtime registration。
- 对 active Run 执行取消和 Cleanup。
- 默认保留或删除日志/Artifact 必须由用户可见选项和 retention policy 决定。
- 禁止删除不属于 Runtime 的用户文件。

---

## 20. M0-M5、测试 Gate 与 Definition of Done

### 20.1 实施顺序

| 阶段 | 依赖 | 必须交付 |
|---|---|---|
| **M0 契约、Source 与信任根** | 无 | RFC 8785/JCS 与 projection/signature corpus、全部 strict unions、RunDraft→SourceAcquisition→RunSpec、DeviceAttestation/Approval/Grant、RuntimeIdentity/Pairing、RevocationBatch、RuntimeAdmissionSnapshot/AdmissionRequirements、Dependency/HardCeilings/ResourceLimit/Network/Redaction/Audit exact contracts、八方法 Runtime Interface、command precondition/target、Cancellation/Timeout/Recovery/Update exact contracts 和 Rust/TypeScript compatibility tests。 |
| **M1 macOS Runtime 控制面** | M0 | Hosted Authorization Authority、stable launcher + signed Rust release、Runtime identity key + pair/re-pair/revoke/reset、single-writer SQLite、epoch-bound hash-chained LocalIPCBinding、strict pre-Grant reservation、atomic admission、signed revocation control inbox、event/audit outbox、Audit/Ledger integrity checkpoints、RuntimeHealth/degraded operation matrix 和独立 activation journal。 |
| **M2 首个完整 untrusted-flow Gate** | M1 | 独立 Design/Execution VZ VM、GuestBootEvidence/boot-bound authenticated vsock、phase EffectGate、ProcessDomainDescriptor/Warden、独立 Secret Broker helper、probe-gated BrowserProvider、EnvironmentFactory；在任何不受信代码/依赖/App/Browser 执行前强制 dependency integrity、Runtime hard limits、per-flow egress、raw quarantine + enforceable redaction、Audit/Ledger checkpoint、process/VM crash containment，并证明 crash/restart 后 admission closed、只读 discovery、绝不 auto-resume。 |
| **M3 完整恢复、Resume 与 Amendment** | M2 | Hosted/local lease lifecycle、execution/control_quiesce_reconcile/control_cleanup transition、first cursor/ack、canonical EffectState/outbox、Checkpoint/Snapshot、split Amendment/Recovery Resume、signed RecoveryDecision、完整 Amendment reapproval、新 Sandbox、Termination、seal/successor Cleanup、terminal repair 和 migration-aware rollback。 |
| **M4 证据、质量与发布 settlement** | M3 | 类型化 Evidence fulfillment、版本化 QualityEvaluation、GitHub/PQL Publication Action、PublicationReceipt、settlement/repair 与全生命周期 failure injection。 |
| **M5 PQL Loop** | M4 | CoverageGap、immutable AssetChangeProposal、PQLReviewDecision、ProjectPackPromotionReceipt、并发 pack conflict 与下一轮回归验证。 |

后续阶段禁止跳过前置阶段的合规 Gate。M1 可以使用最小 Artifact Store，但不得绕过 M0 契约和 Grant 规则。Runtime 内部 R0-R3 只用于本地交付分解，不得替代系统 M0-M5。

每个发布阻断 Gate 必须产出以下记录：

```ts
type VerificationWaiver = RuntimeScopedMeta & {
  waiver_id: string;
  gate_id: string;
  issuer: "fkst-release-authority";
  reason: string;
  excluded_invariant_ids: string[];
  expires_at: ISO8601;
  signature: SignatureBlock;
};

type VerificationGateResult = RuntimeScopedMeta & {
  gate_id: string;
  system_milestone: "M0" | "M1" | "M2" | "M3" | "M4" | "M5";
  runtime_stage?: "R0" | "R1" | "R2" | "R3";
  suite_ids: [string, ...string[]];
  platform: string;
  failpoint_ids: string[];
  asserted_invariant_ids: [string, ...string[]];
  artifact_refs: DigestBoundRef[];
  outcome: "passed" | "failed" | "waived";
  waiver_ref?: DigestBoundRef<"qa.verification-waiver/v1">;
  completed_at: ISO8601;
  signature: SignatureBlock;
};
```

签名/canonicalization、Runtime identity/pairing、Grant/fencing/revocation、Local IPC sequence、Guest channel、dependency integrity、Runtime hard ceilings、network egress、Secret materialization、Browser enforcement、quarantine/redaction、Audit/Ledger durability、crash containment/no-auto-resume 和 Cleanup ownership Gate 禁止 waiver。R0 使用 model/failpoint crash harness；R1 使用真实 adapter/process kill；R2 使用完整取消/恢复/Amendment chaos；R3 使用 update/migration/disk/outbox matrix。

### 20.2 测试矩阵

| 层级 | 场景 | 最低覆盖 |
|---|---|---|
| Contract unit | 每个 Schema/strict union 正反例、未知 version/enum/discriminator、forbidden cross-variant field、RFC 8785 canonical projection/signing bytes | Rust/TypeScript/hosted 共用 `fixtures/rfc8785-v1.json`、`contract-projection-signing-v1.json`、`runtime-protocol-corpus-v1.json`、`runtime-identity-revocation-v1.json`、`runtime-enforcement-audit-v1.json`、`boot-bound-vsock-v1.json`、`runtime-update-compatibility-v1.json` 与 `orchestration-quality-publication-pql-v1.json`；覆盖 DeviceAttestation、pair/re-pair/revoke/reset、IPC/revocation/audit hash chain、SafeErrorDetails bounds、initial/takeover、cancel/timeout/recovery、Verifier/EffectContext、Dependency/Limit/Network/Redaction、Secret Broker、Browser enforcement、Update、Amendment、七类 Quality、Publication reconcile 和 PQL promotion variants。 |
| Source integration | RunDraft、PR synthetic merge、fork PR、merge conflict、exact commit、SourceObject retention、floating ref 变化 | resolver 响应丢失重放、对象 digest 不符、retention 过期均 fail closed。 |
| Authorization unit | signed DeviceAttestation、Runtime identity/pairing epoch、Design/Execution Evidence、signature、request ref、device、TTL、nonce、scope、sequence | challenge/request/run/purpose mismatch、revoked attestation/pairing、旧 identity/pairing epoch、denied Evidence、variant mismatch、旧 approval 和合法签名但替换 payload 全部拒绝。 |
| Authorization integration | Design Approval→strict reservation/exact preimage→Design Grant→Plan→Policy→Execution Approval→strict reservation→Execution Grant→atomic admission | 正常、拒绝、过期、mixed variant、preimage digest mismatch、未激活 reservation、重复 activation、binding/Grant mismatch、device change、stale sequence；验证 idempotency lookup 先于 mutable checks，admission 原子创建 stable environment/empty inventory/CleanupCapability/lease/fencing/effect/outbox。 |
| Policy/PEP | root-qualified 文件、typed command/network/Secret/Browser/VM、phase verifier、resource budget、unknown capability | Design/Execution/bootstrap/control-cleanup context 与 checked digest 正反例；EffectState 全转换、CAS race、uncertain reconcile；worker 无法 check-then-act 绕过。 |
| Plan | PlanCase、DAG、typed PlanAction/strict BrowserAction、root-qualified paths、aggregate envelope、Assertion、EvidenceRequirement、conditional readiness | open action/unknown variant、absolute/bare path、root replacement、orphan Case/Step、跨 Case assertion、required Evidence 缺失必须 fail closed。 |
| Amendment | 新 Step、文件/网络/Secret/权限/预算扩展 | quiesce、旧 Grant revocation、old fence、Cleanup residual、new design sandbox、reapproval/new sandbox。 |
| Runtime protocol | canonical 八方法 probeHealth/reserveLocalLeaseBinding/cancelReservation/submitCommand/getRun/streamEvents/ackEvents/getArtifact、独立 RuntimeTransportControlInbox、split resume、strict cleanup、Snapshot | request/response schema、identity/pairing/boot/session epoch、双向 durable sequence/previous digest/nonce、retired session、同 key 同 digest replay/不同 digest 零状态变更、RevocationBatch sequence/chain/watermark/idempotent ack、first cursor=1、ack conflict/gap、raw artifact fetch denial、旧 Runtime 迟到 completion 不推进；control inbox 不得成为第九个 RuntimeService 方法或承载 command/config。 |
| NyxID adapter | Cloud→Node→loopback、ApprovalEvidence、断线重连、本地认证、路由错误 | 不开放公网端口；不签 Grant、不解释 scope、不读取 Secret。 |
| Sandbox | 每 phase/generation 新 VZ Linux VM、signed boot chain、GuestBootEvidence、BootBoundAuthenticatedVsockSession、ProcessDomainDescriptor、LocalIPCBinding、DependencyAcquisitionPolicy/Receipt、RuntimeHardCeilings、ResourceLimitBinding/Receipt、NetworkFlowReceipt、root identity/mount escape、home denial | bootloader/kernel/initrd/rootfs/agent/nonce/transcript/ephemeral key、sequence/replay/restart、Unix peer/mTLS executable identity、frozen lockfile/integrity/registry redirect/script、VZ+Warden+cgroup+rlimit+storage apply、CPU/memory/disk/process/open-file/time/network/download/quarantine violations、per-flow DNS/redirect/direct-socket/private/metadata denial、root alias/symlink/mount escape 全覆盖；明确不宣称硬件 attestation，禁止回退宿主进程或复用 VM。 |
| Credential | 独立 Secret Broker binding/boot epoch、CredentialLease issue/materialize/release/revoke/reconcile、ProcessDomain/LaunchBinding/ExecutableIdentity/ProcessIdentity、destination/TTL/fence | broker spoof/restart、binary/argv/cwd/domain/PID reuse mismatch 全部拒绝；proxy/env/file custodian、descendant inheritance、core dump/swap/ptrace/logging 场景覆盖；撤销未知形成 blocking residual。 |
| Environment | stable environment、empty inventory activation、version chain/seal barrier、CleanupCapability successor、ExecutableIdentity/launch binding/Warden、Readiness/usage | barrier 前 unsettled effect 阻止 seal；Grant 过期或 control_cleanup rollover 后只可收窄清理；PID reuse、伪造 seal/lineage/capability successor/version conflict 被拒。 |
| Backend | Deterministic、Browser、Codex observation 与 TerminationReceipt | Backend 自报 pass 不能绕过 assertion；cancel ack 不能替代终止证明。 |
| Runner | exit/http/DOM/schema/visual/custom assertions 与 Case 聚合 | required/optional、skip、continue-on-failure、共享 support Step 全覆盖。 |
| Browser E2E | strict BrowserAction/performAction、isolated Profile、BrowserEnforcementCapability、forced proxy/direct-socket denial、点击/DOM/截图 | 全 Chrome process tree 的 IPv4/IPv6 TCP/UDP、QUIC、WebRTC、DoH、DNS rebinding、dynamic network-service、private/metadata/host-loopback bypass；enforcer 中途丢失先断网再终止，required capability 缺失时 reservation 前 deny Browser Plan。 |
| Cancellation/Timeout | design/preparing/ready/executing/evidence/recovery/amendment 各阶段 | cancel/timeout/completion race，子进程、浏览器、端口、CredentialLease 最终 settled。 |
| Recovery | hosted/Runtime/worker 重启、Node 断线、SQLite crash edges、全部 EffectState、重复事件、local recovery latch、execution/control_quiesce_reconcile/control_cleanup rollover、irreconcilable inventory | signed RecoveryDecision variant/TTL/nonce/snapshot/checkpoint/fence/cursor 绑定，Amendment/Recovery Resume 不可混用，strong RuntimeRepairReceipt 完整性；restart 禁止自动 resume。 |
| Cleanup | success/fail/cancel/timeout/lost/restart/amendment、seal barrier、successor capability、partial/not_required/preserved/repair | snapshot ref/version/digest/seal、cleanup lineage/attempt idempotency、missing resource taxonomy、generalized termination、control-only takeover、not_found proof 收敛。 |
| Update | stable launcher、signed manifest/release selection、staged receipt、compatibility set、drain、migration/health/rollback、anti-rollback | stale sequence/nonce、partial set、missing staged receipt、forward/reverse schema window、selection authority 与 rollback failure；recovering/admission/disk health 正确关闭新 Run。 |
| Evidence | enforceable RedactionPolicy/Rule、raw quarantine、RedactionReceipt、durable SanitizedObservation、post-redaction Artifact digest、retention/access/upload | unknown media/rule/action、RE2/detector failure、byte/decompression/archive/time/finding limit、second-pass forbidden class、schema/size failure、partial-output suppression、raw fetch/event exposure、sanitized digest mismatch、上传响应丢失；required Evidence 缺失或脱敏失败禁止 sufficient/publication。 |
| Quality | executed/non-executed、七类 classification、ruleset/input digest | blocked-without-plan、规则升级 supersedes 和 replay 结果可审计。 |
| Publication | GitHub/PQL 独立 Action、response lost、partial failure、repair | dedup、render digest、Artifact allowlist、per-action settlement。 |
| PQL | Gap dedup、immutable proposal/review、stale base、并发 promotion | 未批准资产不可执行；promotion digest/base conflict 不覆盖。 |
| Audit/Ledger integrity | exact AuditEvent variants、event/checkpoint hash chain、LedgerIntegrityCheckpoint/VerificationReceipt、snapshot/health binding | sequence gap、same-sequence different digest、previous mismatch、checkpoint rollback、SQLite/WAL/row/outbox/effect/inventory/nonce root mismatch 全部关闭 admission；禁止清空或跳过坏记录恢复 healthy。 |
| Security | forged attestation/pairing/Grant/RevocationBatch/RecoveryDecision/ReleaseSelection、SafeErrorDetails overflow、replay、stale/wrong-purpose fence、root escape、IPC/vsock spoof、dependency substitution、resource/egress bypass、process-bound Secret bypass、proxy bypass、raw quarantine exposure | 全部 fail closed + exact AuditEvent；任何失败不得改变 reservation/fence/cursor/nonce/watermark 或暴露原始数据。 |

### 20.3 故障注入

故障注入从 M0/R0 开始，而不是等到可恢复编排完成后才补：R0 使用可枚举 transaction/model failpoint；R1 使用真实 adapter、process 和 VM kill；R2 覆盖生命周期 chaos；R3 覆盖 update/migration/disk/outbox。至少自动化注入以下故障：

- Source resolver 已持久化对象但响应丢失，或同 RunDraft 重试得到不同对象 digest。
- Node 在 reservation、CommandAdmissionReceipt 和 command accepted 前后断线；相同 idempotency key 以同/不同 request digest 重试，并验证 lookup 发生在 nonce/fence/cursor/capacity 读取或修改前。
- Runtime 在 admission transaction 的 stable environment、empty inventory、CleanupCapability、lease/fencing、initial Effect/outbox 各逻辑写点崩溃，验证只能全部提交或全部回滚。
- Runtime 在 Design、prepare、execute、evidence、inventory seal、cleanup 和 amendment cleanup 中重启。
- hosted workflow 在收到事件但持久化前/后崩溃，旧 workflow owner 随后恢复。
- 新 generation 生效后收到旧 Runtime 的高 sequence `step_completed` 或 CleanupReceipt；注入错误 purpose 的 execution/control_cleanup FenceTransition 和首个 cursor 非 sequence=1。
- cancel、timeout、amendment_required 和 step_completed 以不同顺序并发到达。
- CredentialLease 已注入但 revoke 响应丢失、Secret Broker 重启后状态不确定，或相同 PID 换 executable/launch binding 后尝试复用 lease。
- Runtime identity key rotation、re-pair、pairing revoke/reset 与旧 Approval/Grant/IPC/RevocationBatch 并发；旧 identity/pairing/session epoch、nonce 和 sequence watermark 不得重新生效。
- LocalIPC peer binary 替换、retired session 新请求、sequence gap/rollback、previous digest mismatch、同 nonce 不同 payload；GuestBootEvidence 中 bootloader/kernel/initrd/rootfs/agent 任一 digest 替换；旧 bootstrap nonce/session transcript 重放；guest ephemeral key、sequence、Runtime/guest restart 或 generation rollover 不匹配。
- RevocationBatch 丢批、乱序、previous digest 替换、watermark rollback、ack 响应丢失和 pairing rollover；Grant effect/Artifact read 必须按 durable watermark fail closed并幂等 ack。
- frozen lockfile 与下载 archive 不一致、registry redirect、浮动 transitive/git dependency、lifecycle script digest 替换；CPU/memory/disk/process/open-file/time/network/download/quarantine hard limit 在副作用边界前后触发；DNS rebinding/direct socket/private/metadata/host-loopback egress 必须被 per-flow enforcement 阻断。
- Backend、guest worker、App、Browser、VM 或 Runtime 在 untrusted flow 任意点崩溃；M2 Gate 必须证明完整 process-domain containment、admission closed、read-only discovery 且不会 auto-resume，完整 Resume 只在 M3 signed RecoveryDecision 后发生。
- Backend 已启动进程域但 command/termination 响应丢失；Effect 分别停在 pending/dispatching/applied/failed_retryable/uncertain/reconciling。
- Browser enforcement capability probe 过期或失败；Chrome 通过 TCP/UDP/QUIC/WebRTC/DoH/service worker/redirect/popup/dynamic network-service 访问未批准公网、RFC1918、link-local、metadata 或 host loopback；active enforcer 中途退出时验证先断网再终止，或 capture raw bytes 在 redaction 前 Runtime 崩溃。
- Artifact 上传部分成功、RedactionReceipt 响应丢失、sanitized digest 不匹配或 raw quarantine 下载被请求。
- Inventory seal barrier 前 Effect 未结算，Cleanup 某资源释放失败、seal/lineage/version/digest 不匹配、capability successor 越权或 `not_found` 无 proof 重试。
- GitHub Check/Product Issue/PQL Gap 已创建但响应丢失。
- PQL Review 后 base Project Pack 并发更新，Promotion 发生 stale-base conflict。
- Grant 在排队期间过期、被撤销或设备发生变化。
- RecoveryDecision 过期/nonce 重放/snapshot 或 cursor 前进，AmendmentResume 与 RecoveryResume 互换。
- AuditEvent 写入、AuditCheckpoint、WAL checkpoint、LedgerIntegrityCheckpoint/VerificationReceipt 各边界崩溃；注入 event gap、same-sequence different digest、checkpoint rollback 与 ledger/audit/outbox/effect/inventory/nonce root mismatch，验证 admission 保持 closed。
- Update staging/WAL checkpoint/SQLite backup 前后崩溃；activation journal 部分写或 fsync 失败；migration marker 每一步、migration commit 后 selection commit 前、selection commit 后 Ledger mirror 前崩溃；launcher crash 而 candidate 存活；`SQLITE_FULL` 同时影响 Ledger 与 journal；previous 可读但不可写；reverse migration 半完成；release sequence 回退、partial compatibility set 和 health gate 失败。

系统必须通过 fenced Snapshot、canonical EffectState ledger、seal barrier、dedup key、immutable Receipt、purpose-specific FenceTransition 和 signed RecoveryDecision 收敛，禁止通过“重新跑整个 Run”掩盖一致性问题。

### 20.4 Definition of Done

以下条件全部满足后，v1 才可以声明完成：

1. [ ] 根契约均有可执行 Schema、validator、命名 projection、RFC 8785/JCS canonical digest/signature 和兼容测试；Rust/TypeScript/hosted 对 `fixtures/rfc8785-v1.json` 及 Runtime protocol/signing/vsock/update corpora 的 canonical bytes、digest、signature 与 rejection code 完全一致，并共同执行 `fixtures/orchestration-quality-publication-pql-v1.json` 的 Amendment、Quality、Publication reconcile 和 PQL conflict cases。
2. [ ] RunDraft、SourceAcquisition、SourceRevision 和 RunSpec 分层明确；PR 使用可获取的 immutable synthetic merge object，非 PR 使用 exact SHA；Source 变化创建新 Run。
3. [ ] DesignApprovalEvidence 与 ExecutionApprovalEvidence 是严格不同的 variant；用户批准 scope/Plan，Hosted Authorization Authority 签发 Design Grant 和 Execution Grant，NyxID 只提供证明和传输。
4. [ ] Design/Execution reservation 与 authorization preimage 是严格 variant；Authority/Runtime 对同一 exact JCS preimage 重算 digest。两种 Grant 都绑定 Runtime 签名 LocalLeaseBinding；reservation 直到 command admission 都不 fence active generation，且幂等 lookup 先于 mutable reservation/fence/cursor/capacity/nonce 检查。
5. [ ] StructuredPlan 包含 typed strict PlanAction/BrowserAction、root-qualified path、PlanCase、稳定 Assertion 和 EvidenceRequirement 关联；Plan 先于 Execution Approval，批准后不可原地修改。
6. [ ] 超出 envelope 的动作完成 quiesce、旧 Grant revocation/fencing、Cleanup、amendment Design、Policy/Approval、新 Grant 和新 Sandbox 全链路。
7. [ ] `apps/hosted-control-plane` 与 `apps/local-qa-runtime` 可以独立构建和发布；testing packages 不依赖 apps。
8. [ ] macOS Runtime 由签名 user LaunchAgent 启动 stable launcher，再按 signed monotonic RuntimeReleaseSelection 选择 Rust Supervisor；不安装 root LaunchDaemon，并具备 recovering/admission/capacity/disk health、staged update、migration compatibility、anti-rollback、A/B rollback 和卸载流程。
9. [ ] RuntimeService 只暴露八个 canonical Interface：probeHealth/reserveLocalLeaseBinding/cancelReservation/submitCommand/getRun/streamEvents/ackEvents/getArtifact；reservation cancellation 不替代 fenced CancelCommand。控制接口只监听 loopback/Unix socket并以 LocalIPCBinding + peer ExecutableIdentity 认证，生产环境不存在 `auth_method=none`。
10. [ ] 每次 Design/Execution command admission 原子创建 retry-stable environment id、empty inventory root、完整 signed CleanupCapability、active LocalExecutionLease、strict initial/takeover predecessor、FenceTransition、initial Effect/outbox；首次 generation 不伪造 predecessor inventory，bootstrap context 不依赖 PlanStep/PreparedEnvironment，随后才创建各自新的 VZ VM/overlay。
11. [ ] Sandbox 默认不能读取用户主目录、Keychain、个人浏览器 Profile 和未批准目录；path/symlink/mount escape 被阻断。
12. [ ] 文件、命令、网络、Secret、Browser、VM 和资源动作只能通过 Rust EffectGate.perform 执行；Design/Execution/Cleanup verifier 与 EffectContext 是 strict phase variants，checked digests 完整，EffectRecord 只使用 canonical pending/dispatching/applied/denied/failed_retryable/failed_final/uncertain/reconciling/suppressed/settled 状态。
13. [ ] 外部 Fork PR 默认没有长期 Secret、写 token、生产环境或私网访问；独立 Secret Broker helper 通过 broker binding/boot epoch 与受认证 IPC 工作，materialization 必须绑定 CredentialLease + ProcessDomainDescriptor + ProcessLaunchBinding + ExecutableIdentity + actual ProcessIdentity；Supervisor/Ledger 只保存 opaque refs/digests/receipts，proxy/env/file 模式的明文 custodian、继承和擦除规则均通过测试。
14. [ ] EnvironmentFactory 维护 append-only inventory、seal barrier/InventorySealReceipt 和 CleanupCapability successor chain；Process Warden 使用 ExecutableIdentity + ProcessLaunchBinding + PID/start token/ownership domain，端口、mount、IPC/vsock、proxy、Credential、quarantine、usage 和 Readiness 均进入 inventory/Receipt。
15. [ ] Deterministic、Browser、Codex Backend 实现统一接口；Backend 只返回 observation。
16. [ ] `testing-runner` 根据结构化 assertion 生成 CaseResult；Codex 自报结论不能改变 Pass/Fail。
17. [ ] BrowserProvider 只接受 strict BrowserAction 并通过 performAction 执行；只有未过期 BrowserEnforcementCapability 能证明完整 Chrome process tree 的 IPv4/IPv6 TCP/UDP direct-socket denial 时才广告 Browser capability。宿主 Chrome 使用临时 Profile、forced proxy、签名 BrowserNetworkEnforcementReceipt 和 opaque session；enforcement 丢失先断网再终止，worker 不获得 CDP endpoint/token。
18. [ ] 成功、失败、取消、超时、失联、Runtime 重启和 Amendment 均通过 generalized TerminationTargetScope、signed RecoveryDecision、split Amendment/Recovery Resume 和 purpose-specific fenced Cleanup 收敛。
19. [ ] Cleanup 使用 capability/current-or-successor、sealed snapshot ref/lineage/version/digest、InventorySealReceipt 和 cleanup lineage/attempt idempotency，只清理本 Run 资源，并完整区分 missing/unknown/active residual 与合法 preserved resource。
20. [ ] execution、cleanup、evidence、publication、final quality 五类 Outcome 独立持久化；terminal 表示 settled，terminal 后 repair 不重开测试或改写 QualityEvaluation。
21. [ ] 所有原始观察先进入 Runtime-only RawQuarantineArtifact；RedactionReceipt 成功后持久化 SanitizedObservation，ArtifactPointer 只记录 post-redaction digest。EvidenceManifest 逐项结算，缺失/脱敏失败/digest mismatch 禁止 sufficient/publication。
22. [ ] `quality-evaluation` 支持 executed/non-executed 输入，绑定完整 input set 与 rule set，并区分 product、test、coverage、environment、flaky、policy 和 insufficient evidence。
23. [ ] Publication 只消费 QualityEvaluation 和 Artifact allowlist；GitHub 与 PQL 是独立 Action，均有 rendered digest、dedup key、attempt、reconcile 和 Receipt。
24. [ ] 产品 Issue 只有在可复现、Evidence sufficient 且排除测试/环境/Flaky 后才创建。
25. [ ] 测试资产问题进入 PQL；AssetChangeProposal 不可原地批准，只有 PQLReviewDecision 与 ProjectPackPromotionReceipt 可产生后续 Run 使用的新 Project Pack。
26. [ ] hosted、Runtime、worker 或 transport 重启后先进入 local recovery latch/read-only discovery；收到 signed RecoveryDecision 后才通过 execution/control_quiesce_reconcile/control_cleanup FenceTransition、Hosted/Local lease、PredecessorFencingRecord、Checkpoint/Snapshot/cursor/ack、RecoveryLedger、canonical EffectState、seal、Warden/VM/Chrome/Secret/quarantine reconcile 和 strong RuntimeRepairReceipt 恢复；Runtime restart 不会自动继续测试或自行取得 takeover。
27. [ ] 同一 Run 重放先命中 idempotency lookup，不会在冲突检查前消费 nonce或改变 reservation/fence/cursor/capacity；不会重复启动服务、执行 Step、注入 CredentialLease、seal inventory、Cleanup、上传 Artifact 或创建外部对象。
28. [ ] Source、Approval、Grant、fence、Local PEP、Credential、Termination、Cleanup、Quality、Publication、PQL Review/Promotion 和 repair 均可审计，审计中不含 Secret。
29. [ ] 测试矩阵和故障注入场景在 CI 或受控真实设备测试中通过，并保存带 gate_id、suite、platform、failpoint、invariant、artifact 和 waiver 的 VerificationGateResult；签名/fencing/Guest/Secret/Browser/Ledger/Cleanup Gate 不允许 waiver。
30. [ ] POC 之外的能力只在对应验收通过后标记为 implemented；文档和 UI 不得把目标架构误报为已完成能力。
31. [ ] cancel、timeout、amendment 和 completion race 的优先级有自动化测试，旧 fence 事件不会复活 Run。
32. [ ] Grant 过期或撤销后仍可使用最小 CleanupCapability 清理既有资源；higher-fence 路径必须使用 control_cleanup lease、signed proof 和收窄 successor capability，不能创建资源、恢复执行或取得新 CredentialLease。
33. [ ] Cleanup/Publication residual 在重试预算耗尽后有明确 repair responsibility、告警和不可变 Receipt。
34. [ ] blocked-without-plan、Design/Execution denial 和 cancel-before-execution 均能产生 NonExecutedQualityEvaluation，不要求伪造 Plan 或 Evidence。
35. [ ] Mermaid、Excalidraw、SVG、PNG、DESIGN 与 SPEC 对 Authority、Approval、PEP、Amendment、Publication 和 PQL 语义一致。
36. [ ] 每个 activated generation 的第一个 RuntimeEvent 恰为 sequence=1 的 command_accepted；stream from_first、after 和 ackEvents 语义通过断线/重复/gap 测试。
37. [ ] LocalIPCBinding、GuestBootEvidence、BootBoundAuthenticatedVsockSession、ProcessDomainDescriptor、ExecutableIdentity 和 ProcessLaunchBinding 能阻止 loopback/guest spoof、boot/transcript/session replay、binary replacement、PID reuse 与 process-bound Secret 绕过，并明确该 guest 机制不是硬件 attestation。
38. [ ] RuntimeUpdateManifest、UpdateStagingInventory、RuntimeActivationRequest、LauncherActivationJournalEntry、RuntimeActivationHealthEvidence/Result、staged receipt、reader/writer migration compatibility、anti-rollback watermark 和 RuntimeReleaseSelection 均可验签、审计并通过失败注入；update 使用 RuntimeErrorEnvelope(phase=update)。
39. [ ] DeviceAttestation 以 provider signature 绑定 challenge/request/run/purpose/user/device、Runtime identity/pairing epoch、assurance、TTL/nonce；ApprovalEvidence 与两类 Grant 使用可解析的 `device_attestation_ref`，不存在 digest-only attestation authorization。
40. [ ] RuntimeIdentityStatement 与 RuntimePairingChallenge/Receipt 覆盖初始 pairing、key rotation continuity、re-pair、revocation 和 reset；旧 identity/pairing epoch 的 Grant、attestation、reservation、IPC session 与 revocation ack 全部 fail closed。
41. [ ] LocalIPC request/response 使用独立 durable sequence + previous digest + nonce chain，retired session 不接受新消息；RuntimeTransportControlInbox 不计入八方法 RuntimeService且只接收 hash-linked Grant/Artifact RevocationBatch，ack 对断线/重启幂等。
42. [ ] 任一 untrusted flow 之前均已完成 frozen-lockfile dependency integrity、signed RuntimeHardCeilings、ResourceLimitBinding apply、per-flow NetworkFlowReceipt、raw quarantine 与可执行 RedactionPolicy；任一 enforcement 缺失或超限都 fail closed。
43. [ ] AuditEvent 是 exact strict union并形成无间隙 hash chain；AuditCheckpoint 与 LedgerIntegrityCheckpoint/VerificationReceipt 覆盖 SQLite/WAL、audit、outbox、effect、inventory 和 nonce/sequence watermarks，integrity failure 关闭 admission且不能通过清空历史恢复。
44. [ ] M2 Gate 在 Design/Execution 的第一段不受信代码前同时验证 dependency、hard limits、egress、quarantine/redaction、audit 和 process/VM crash containment；任一 crash/restart 只进入 admission-closed read-only discovery，完整 Resume/Amendment 仅在 M3 通过 signed RecoveryDecision 和重新授权执行。

### 20.5 需求追踪矩阵

| 锁定要求 | Schema / Interface | 状态/规则 | DoD |
|---|---|---|---|
| fkst-hosted monorepo、两个 app 独立部署、testing packages | §1、§2 | module boundary tests | #7 |
| RunDraft、SourceAcquisition、immutable source、RunSpec | §4、`DigestBoundRef` | `source_resolving` | #2 |
| Hosted Authorization Authority、strict reservation/preimage 与原子 admission | DeviceAttestation、Approval/Grant、AdmissionRequirements、LocalLeaseBinding、CommandAdmissionReceipt | §5、§8、§16 | #3-4、#10、#27、#39 |
| Runtime identity、pair/re-pair/revoke/reset 与 epoch binding | RuntimeIdentityStatement、RuntimePairingChallenge/Receipt、GrantDeviceBinding | §5、§19 | #39-40 |
| NyxID 只提供证明、Credential Broker 适配与传输 | Runtime protocol、NyxID Adapter | §8、§17 | #3、#9、#13、#39-41 |
| Typed PlanAction/BrowserAction、root-qualified path、PlanCase/Assertion/Evidence | `StructuredPlan`、`EvidenceManifest` | §6、§10-§11 | #5、#16-17、#21 |
| macOS stable launcher、signed release/update、launchd health/admission | RuntimeHealth、RuntimeUpdateManifest/Receipt、RuntimeReleaseSelection | §19 | #8-9、#38 |
| Local Design/Execution Sandbox、phase verifier 与 Local PEP | EnvironmentFactory、VerifierInput/EffectContext、EffectGate、BrowserProvider | §9、§10、§17 | #10-14、#17、#37、#42、#44 |
| Dependency integrity、hard limits 与逐流 egress | DependencyAcquisitionPolicy/Receipt、RuntimeHardCeilings、ResourceLimitBinding/Receipt、NetworkFlowReceipt | §9、§17、M2 Gate | #42、#44 |
| Process-bound Credential 最小租约 | CredentialLease/Receipt、ExecutableIdentity、ProcessLaunchBinding、SecretBrokerClient | §9、§17 | #13-14、#27、#37 |
| Plan Amendment hard-revoke 与 split Resume | PlanAmendmentRequest/PlanAmendment、AmendmentResumeCommand、RecoveryResumeCommand | §7、§8、§13、§16 | #6、#18、#26、#31 |
| Runtime fencing、cursor/ack 与 signed recovery | ExecutionFence、FenceTransition、PredecessorFencingRecord、RuntimeCommand/Event、RecoveryDecision、WorkflowCheckpoint | §8、§13、§16 | #18、#26、#31、#36 |
| testing-runner 决定 Pass/Fail | BackendObservation、AssertionResult、CaseResult | §10 | #15-16 |
| Generalized Termination、inventory seal 与补偿 Cleanup | TerminationTargetScope/Receipt、InventorySealReceipt、CleanupCapability successor、CleanupReceipt/Residual | §9-§11、`cleaning_up`/repair | #18-19、#32-33 |
| 五类 Outcome 与 settlement | RunOutcomes、RunSettlement、RepairOperation | §14 | #20、#33-34 |
| Quality 与精确路由 | QualityEvaluation unions、PublicationPlan/Receipt | §12、§18 | #22-24、#34 |
| PQL Review 与 Promotion | AssetChangeProposal、PQLReviewDecision、ProjectPackPromotionReceipt | §12、§18 | #25 |
| Raw quarantine、enforceable redaction 与 sanitized Artifact | RedactionPolicy/Rule、RawQuarantineArtifact、RedactionReceipt、SanitizedObservation、ArtifactPointer | §10-§11、§17 | #21、#37、#42、#44 |
| Local IPC 与 boot-bound guest authentication | RuntimeIdentityStatement/PairingReceipt、LocalIPCBinding/LocalRequestAuthentication、GuestBootEvidence、BootBoundAuthenticatedVsockSession、ProcessDomainDescriptor、ExecutableIdentity、ProcessLaunchBinding | §8-§9、§17、§19 | #9、#14、#37、#40-41 |
| Grant/Artifact access 撤销 delivery | RevocationFact/Batch、RevocationDeliveryReceipt、RuntimeTransportControlInbox | §5、§8、§11 | #32、#41 |
| Exact Audit 与 Ledger integrity | AuditEvent、AuditCheckpoint、LedgerIntegrityCheckpoint/VerificationReceipt、SafeErrorDetails | §15、§17、§19 | #28-29、#43-44 |
| 图与规范语义同步 | Mermaid/Excalidraw/SVG/PNG | §0、DESIGN §1 | #35 |

---

本规范的实现应按系统 M0-M5 逐步收敛。Runtime 内部 R0-R3 只表达本地交付增量。任何阶段可以缩小功能范围，但不得绕过授权、源码冻结、Sandbox enforcement、Runner assertion、Cleanup、Outcome 分离和幂等这些系统不变量。
