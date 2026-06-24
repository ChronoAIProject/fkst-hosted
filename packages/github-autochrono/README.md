# github-autochrono composed 包

这是一个 **composed 包**（适配/wiring 层）,把 `github-proxy` 与 `autochrono` 组合起来,而两个包互不认识:`github-proxy` 只发布 GitHub 实体变化、消费评论请求;`autochrono` 只消费自己的 `issue` 契约、产出自己的 `reply` 契约。本包的 glue 部门是唯一同时引用 `github-proxy.*` 与 `autochrono.*` 队列的层,耦合集中于此。

`fkst.toml` 的 `[event_deps]` 声明它组合的兄弟包(`github-proxy`、`autochrono`),作为标准测试入口拼装组合 conformance 的唯一来源。因为 glue 部门引用跨包命名空间,本包不做单根 conformance(只在组合图里有效)。

链路:

```text
github-proxy.github_entity_changed
  -> autochrono.issue
  -> consensus.proposal
  -> consensus.consensus_reached
  -> autochrono.reply
  -> github-proxy.github_issue_comment_request
```

`departments/inbound_glue` 只把 GitHub issue 事件映射为 `autochrono.issue.v1`,忽略 PR。`autochrono` 把 issue 映射为 `consensus.proposal.v1`,由 `consensus` 产出 `consensus.consensus_reached.v1`,仅在 approve 时继续产出 `autochrono.reply.v1`。`departments/outbound_glue` 把 `autochrono.reply.v1` 映射为 `github_issue_comment_request`,贯穿 `issue_number`/`body`/`dedup_key`/`source_ref`。`core.lua` 只含纯映射函数,`tests/core_test.lua` 只测这些函数,不依赖组合图或运行时 PATH。

测试(标准入口):

```sh
scripts/run.sh test            # 全包:flat 单根 + composed 跳单根+单测 + 组合 conformance
scripts/run.sh test-composed   # 只跑组合图 conformance(按 [event_deps] 递归 union github-proxy + autochrono + consensus + 本包)
```

⟦AI:FKST⟧
