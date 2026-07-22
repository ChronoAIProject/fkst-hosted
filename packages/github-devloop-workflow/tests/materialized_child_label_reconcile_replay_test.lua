local actions = require("core.materialize.actions")
local base_ids = require("devloop.base_ids")
local core = require("core")
local devloop_base = require("devloop.base")
local devloop_logging = require("devloop.logging")
local digest = require("core.digest")
local marker = require("core.marker")
local materialization = require("core.materialization")
local materialize_reconcile = require("materialize_reconcile")
local testing = require("testkit.testing")
local t = fkst.test

local repo = "owner/repo"
local origin_issue = 42
local child_issue = 108
local origin = base_ids.proposal_id(repo, origin_issue)

local function blueprint()
  return {
    schema = "fkst.workflow.v1",
    id = "repair-workflow",
    version = "1",
    summary = "Repair fixture.",
    applies_when = "The workflow is selected.",
    steps = {
      {
        id = "implement",
        title = "Implement",
        content = { kind = "static", intent = "Implement the requested change." },
      },
    },
  }
end

local function spec()
  return {
    title = "Implement the requested change",
    body = "Implement the requested change.",
  }
end

local function entry()
  return materialization.created_entry(
    origin,
    digest.blueprint_digest(blueprint()),
    blueprint().steps[1],
    materialization.EMPTY_PREDECESSOR_REF_DIGEST,
    spec(),
    child_issue
  )
end

local function comment(body)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-07-23T00:00:00Z",
  }
end

local function parent_issue()
  local blueprint_body = marker.build_blueprint_marker(origin, blueprint().id, digest.blueprint_digest(blueprint()))
  local e = entry()
  local ledger_body = marker.build_materialization_marker(
    origin,
    e.blueprint_digest,
    e.slot,
    e.predecessor_ref_digest,
    e.gen_contract_digest,
    e.gen_spec_digest,
    e.child_dedup,
    tostring(child_issue),
    "created"
  )
  return {
    title = "Workflow origin",
    body = "Run the workflow.",
    state = "OPEN",
    labels = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    comments = { comment(blueprint_body), comment(ledger_body) },
    repo = repo,
    number = origin_issue,
  }
end

local function child(labels)
  local lineage = marker.build_lineage_header(origin, digest.blueprint_digest(blueprint()), "implement")
  return {
    number = child_issue,
    title = spec().title,
    body = lineage .. "\n\n" .. spec().body
      .. "\n\n<!-- fkst:github-proxy:issue-create:" .. entry().child_dedup .. " -->",
    state = "OPEN",
    labels = labels or {},
    comments = {},
    author_login = "fkst-test-bot",
  }
end

local function event()
  return {
    queue = "github-devloop-workflow.workflow_materialization_tick",
    payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
    ts = "2026-07-23T00:00:00Z",
  }
end

local function run_pass(current_child)
  devloop_base.configure_trusted_bot_login("fkst-test-bot")
  local dept = require("workflow.saga").department({
    consumes = { "workflow_materialization_tick" },
    produces = {
      "github-proxy.github_issue_create_request",
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    },
    stall_window = "2m",
  }, materialize_reconcile.handlers(core, {
    deps = {
      read_repo = function() return repo end,
      list_open_issues = function() return { { number = origin_issue } } end,
      read_issue = function() return parent_issue() end,
      verify_issue_claim = function() return true end,
      load_blueprint = function()
        return { path = "repair-workflow.json", blueprint = blueprint() }
      end,
      read_created_issue = function(read_repo, number)
        t.eq(read_repo, repo)
        t.eq(number, tostring(child_issue))
        return current_child
      end,
      session_work_labels = function()
        return { "fkst-dev", "fkst-security" }
      end,
      child_status = function()
        return "running"
      end,
      release_done_claim = function() return true end,
    },
  }))

  local decisions = {}
  local original_log = devloop_logging.log_cas_decision
  devloop_logging.log_cas_decision = function(dept_name, proposal_id, current, from_state, to_state, outcome, reason)
    decisions[#decisions + 1] = {
      dept = dept_name,
      proposal_id = proposal_id,
      from_state = from_state,
      to_state = to_state,
      outcome = outcome,
      reason = reason,
    }
    return original_log(dept_name, proposal_id, current, from_state, to_state, outcome, reason)
  end
  local old_with_lock = with_lock
  with_lock = function(_key, locked) return locked() end
  local ok, result = pcall(function()
    return testing.run_fake(dept, event())
  end)
  with_lock = old_with_lock
  devloop_logging.log_cas_decision = original_log
  if not ok then
    error(result, 0)
  end
  return result, decisions
end

local function raises_to(result, queue)
  local found = {}
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == queue then
      found[#found + 1] = raised
    end
  end
  return found
end

local function decision_with_outcome(decisions, expected)
  for _, decision in ipairs(decisions or {}) do
    if decision.outcome == expected then
      return decision
    end
  end
  return nil
end

return {
  test_restart_replay_converges_around_label_effect_visibility = function()
    local first, first_decisions = run_pass(child({}))
    local first_labels = raises_to(first, "github-proxy.github_issue_label_request")
    t.eq(#first_labels, 1)
    t.eq(#raises_to(first, "github-proxy.github_issue_create_request"), 0)
    t.eq(#raises_to(first, "github-proxy.github_issue_comment_request"), 0)
    t.eq(first_labels[1].payload.add_labels[1], "fkst-dev")
    t.eq(#first_labels[1].payload.remove_labels, 0)
    t.is_nil(first_labels[1].payload.claim)
    t.is_true(decision_with_outcome(first_decisions, "applied(repaired-missing-work-label)") ~= nil)

    local before_visible, replay_decisions = run_pass(child({}))
    local replay_labels = raises_to(before_visible, "github-proxy.github_issue_label_request")
    t.eq(#replay_labels, 1)
    t.eq(replay_labels[1].payload.dedup_key, first_labels[1].payload.dedup_key)
    t.is_true(decision_with_outcome(replay_decisions, "applied(repaired-missing-work-label)") ~= nil)

    local after_visible, visible_decisions = run_pass(child({ "bug", "fkst-dev" }))
    t.eq(#raises_to(after_visible, "github-proxy.github_issue_label_request"), 0)
    t.eq(#raises_to(after_visible, "github-proxy.github_issue_create_request"), 0)
    t.eq(#raises_to(after_visible, "github-proxy.github_issue_comment_request"), 0)
    t.is_true(decision_with_outcome(visible_decisions, "skip-idempotent(work-label-present)") ~= nil)

    local periodic = run_pass(child({ "fkst-dev" }))
    t.eq(#periodic.raises, 0)
  end,

  test_skipped_repair_emits_structured_scope_outcome = function()
    local result, decisions = run_pass(child({ "fkst-security" }))
    t.eq(#raises_to(result, "github-proxy.github_issue_label_request"), 0)
    local skipped = decision_with_outcome(decisions, "skip-conflicting-work-label")
    t.is_true(skipped ~= nil)
    t.eq(skipped.proposal_id, base_ids.proposal_id(repo, child_issue))
    t.eq(skipped.from_state, "materialized-child-label")
    t.eq(skipped.to_state, "work-label")
  end,

  test_current_created_entries_ignore_noncurrent_generated_fact = function()
    local created = entry()
    local generated = entry()
    generated.state = "generated"
    generated.child_issue = nil
    local current = actions.current_created_entries({ generated, created })
    t.eq(#current, 1)
    t.eq(current[1].child_issue, tostring(child_issue))
  end,
}
