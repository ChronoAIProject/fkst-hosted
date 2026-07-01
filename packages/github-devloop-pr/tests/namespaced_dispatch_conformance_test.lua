local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local conv_reconcile = require("devloop.convergence.reconcile")
local t = h.t
local core = h.core
local conformance = require("testkit.namespaced_dispatch_conformance")
local m_mq = require("devloop.merge_queue")

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/comment_handoff/main.lua", "departments.comment_handoff.main"),
  load_department("departments/fix/main.lua", "departments.fix.main"),
  load_department("departments/liveness_scan/main.lua", "departments.liveness_scan.main"),
  load_department("departments/merge/main.lua", "departments.merge.main"),
  load_department("departments/merge_queue/main.lua", "departments.merge_queue.main"),
  load_department("departments/observe_pr/main.lua", "departments.observe_pr.main"),
  load_department("departments/reconcile/main.lua", "departments.reconcile.main"),
  load_department("departments/review_loop/main.lua", "departments.review_loop.main"),
  load_department("departments/review_meta/main.lua", "departments.review_meta.main"),
  load_department("departments/review_pr/main.lua", "departments.review_pr.main"),
  load_department("departments/review_result/main.lua", "departments.review_result.main"),
})

local function review_proposal_id(version, head_sha)
  return devloop_base.pr_review_proposal_id("owner/repo", 7, version or h.reviewing().version, head_sha or "def456")
end

local function review_reached(extra)
  local value = h.review_reached(extra)
  if value.angle_results == nil then
    value.angle_results = {
      { angle = "minimal", verdict = "approve" },
      { angle = "structural", verdict = "approve" },
      { angle = "delete", verdict = "approve" },
    }
  end
  return value
end

local function review_unresolved(extra)
  return h.review_unresolved(extra)
end

local function merge_ready()
  return h.merge_ready()
end

local function timeout_reconcile()
  return conv_reconcile.build_devloop_timeout_reconcile_payload(core, {
    from_state = "merge-ready",
  }, {
    version = h.merge_ready().version .. "/timeout/merge-ready/1",
  }, "github-devloop/issue/owner/repo/42", { kind = "external", ref = "owner/repo#pr/7" }, 1)
end

local function payload_for_queue(_path, queue)
  local payloads = {
    ["consensus.consensus_converge"] = review_unresolved(),
    ["consensus.consensus_reached"] = review_reached(),
    ["github-proxy.github_comment_written"] = {
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      comment_id = "123456",
      request_dedup_key = "github-devloop/issue/owner/repo/42/comment/reviewing/v1",
      dedup_key = "github-devloop/issue/owner/repo/42/comment/reviewing/written/123456",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      handoff = {
        kind = "github-devloop.reviewing",
        proposal_id = "github-devloop/issue/owner/repo/42",
        pr_number = 7,
        version = h.reviewing().version,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
    },
    ["github-proxy.github_entity_changed"] = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "owner/repo#pr#7@2026-06-03T01:02:03Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    devloop_pr_observe_redrive = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "owner/repo#pr#7@2026-06-03T01:02:03Z",
      source = "liveness-scan",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    devloop_fix_reconcile = h.fix_reconcile(),
    devloop_fixing = h.fixing(),
    devloop_liveness_tick = { schema = "github-devloop.tick.v1" },
    devloop_merge_queue_tick = m_mq.merge_queue_tick_payload(core, "owner/repo", 6, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      pr_number = 7,
      version = h.reviewing().version,
      review_proposal_id = review_proposal_id(),
      review_dedup_key = "consensus:" .. review_proposal_id() .. "/review",
      reviewed_head_sha = "def456",
      head_sha = "def456",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }),
    devloop_merge_ready = merge_ready(),
    devloop_observe_pr = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "liveness-scan/owner/repo/pr/7",
      source = "liveness-scan",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    devloop_review_meta = h.review_meta_event(),
    devloop_review_reconcile = h.review_reconcile(),
    devloop_reviewing = h.reviewing(),
    devloop_timeout_reconcile = timeout_reconcile(),
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-devloop-pr: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "github-devloop-pr",
      package_root = "packages/github-devloop-pr",
      departments = departments,
      payload_for_queue = payload_for_queue,
    })
  end,
}
