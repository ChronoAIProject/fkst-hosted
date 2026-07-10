local devloop_base = require("devloop.base")
local payloads_builders = require("devloop.payloads.builders")
local ci_repair_attempts = require("core.ci_repair_attempts")
local config = require("devloop.config")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local m_builders = require("devloop.markers.builders")
local opts = h.opts
local fixing = h.fixing
local run_fix = h.run_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise
local mock_existing_fix_worktree = h.mock_existing_fix_worktree

local function mock_real_write_env_reads()
  for _ = 1, 4 do
    mock_write_env("1")
  end
end

local function with_codex_runs(running, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = running or {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
end

local function queue_pr_comments(pr_number, issue_number, version, head_sha)
  local proposal_id = "github-devloop/issue/owner/repo/" .. tostring(issue_number)
  local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", pr_number, version, head_sha)
  local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
  return {
    m_builders.pr_origin_marker(proposal_id, tostring(issue_number), "devloop-owner-repo-" .. tostring(pr_number), version, "dev"),
    core.state_marker(proposal_id, "merge-ready", version),
    m_builders.merge_ready_marker(proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha),
    m_builders.review_result_marker(review_proposal_id, proposal_id, "approve", review_dedup_key),
  }
end

local function json_comment(comment)
  return '{"body":"' .. h.json_string(comment) .. '","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-03T01:00:00Z"}'
end

local function json_comments(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, json_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function mock_merge_queue_list(pr_numbers)
  local rendered = {}
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(rendered, '{"number":' .. tostring(number) .. ',"headRefName":"devloop-owner-repo-' .. tostring(number) .. '","headRefOid":"def456","baseRefName":"dev","state":"OPEN"}')
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=dev&per_page=100'", {
    stdout = "[[" .. table.concat(rendered, ",") .. "]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_queue_pr(pr_number, issue_number, version, head_sha)
  t.mock_command("gh pr view '" .. tostring(pr_number) .. "' --repo 'owner/repo' --json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = '{"number":' .. tostring(pr_number)
      .. ',"headRefName":"devloop-owner-repo-' .. tostring(pr_number)
      .. '","headRefOid":"' .. tostring(head_sha)
      .. '","baseRefName":"dev","baseRefOid":"abc123","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"merged":false,"mergedAt":"","comments":['
      .. json_comments(queue_pr_comments(pr_number, issue_number, version, head_sha))
      .. '],"labels":[],"headRepository":{"nameWithOwner":"owner/repo","owner":{"login":"owner"}},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function base_review_identity()
  local base = fixing()
  return {
    proposal_id = base.proposal_id,
    pr_number = base.pr_number,
    review_proposal_id = base.review_proposal_id,
    review_dedup_key = base.review_dedup_key,
    reviewed_head_sha = base.reviewed_head_sha,
    source_ref = base.source_ref,
    base_version = base.version, -- fix_round 1
  }
end

local function fixing_at_round(round, review_fact_extra)
  local id = base_review_identity()
  local version = id.base_version
  for _ = 2, round do
    version = core.next_fix_version(version)
  end
  local review_fact = {
    review_proposal_id = id.review_proposal_id,
    review_dedup_key = id.review_dedup_key,
    reviewed_head_sha = id.reviewed_head_sha,
    predecessor_set = "none",
    gate_failure_excerpt = "own-ci-red",
  }
  for key, value in pairs(review_fact_extra or {}) do
    review_fact[key] = value
  end
  local event = payloads_builders.build_devloop_fixing_payload({
    proposal_id = id.proposal_id,
    impl_version = version,
  }, id.pr_number, review_fact, id.source_ref)
  event.work_unit_key = payloads_builders.fixing_work_unit_key(event)
  return event
end

local function mock_speculative_predecessor_drift(event, feedback_comments)
  local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
  local merge_ready_version = core._strip_latest_fix_version_suffix(event.version)
  mock_bot_env()
  mock_real_write_env_reads()
  mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, feedback_comments, branch, event.version)
  -- PR comments carry the merge-ready fact (so the PR is a member of the speculative merge
  -- queue) plus the current fixing feedback markers.
  local pr_comments = {
    m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", merge_ready_version),
    m_builders.merge_ready_marker(event.proposal_id, event.pr_number, merge_ready_version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    m_builders.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
  for _, comment in ipairs(feedback_comments) do
    table.insert(pr_comments, comment)
  end
  local own_ci_rollup = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"test","headSha":"def456"}]'
  mock_pr_fix(pr_comments, branch, "def456", nil, nil, nil, nil, own_ci_rollup)
  h.mock_required_check_runs_for("def456", "failure", "owner/repo")
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  mock_existing_fix_worktree(branch, "def456")
  -- The current merge queue holds PR 6 ahead of PR 7, so the current predecessor set differs
  -- from the recorded "none" -> predecessor-set mismatch -> speculative refix.
  mock_merge_queue_list({ 6 })
  mock_merge_queue_pr(6,
    41,
    "ready/consensus-github-devloop/issue/owner/repo/41/2026-06-03T00-00-00Z",
    "abc999"
  )
end

local function run_speculative_refix(event, feedback_comments, name)
  mock_speculative_predecessor_drift(event, feedback_comments)
  local result
  with_codex_runs({}, function()
    result = run_fix(event, opts(name, { FKST_GITHUB_WRITE = "1" }))
  end)
  return result
end

-- Speculative fixing feedback via the merge-gate marker. ci_failure_key set => own-CI-red
-- (repair_input == "ci-failure"); ci_failure_key nil => speculative merge conflict
-- (repair_input == "review-feedback").
local function speculative_feedback_comments(event, ci_failure_key)
  return {
    core.state_marker(event.proposal_id, "fixing", event.version),
    m_builders.merge_gate_marker(event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      nil,
      ci_failure_key ~= nil and "own-ci-red" or "speculative-merge-conflict",
      "none",
      ci_failure_key
    ),
  }
end

local function find_fixing_handoff(result)
  return find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return type(payload.handoff) == "table" and payload.handoff.kind == "github-devloop.fixing"
  end)
end

local function find_attempt_fact(result)
  return find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find("ci-repair-attempt:v1", 1, true) ~= nil
  end)
end

return {
  test_stale_fixing_handoff_with_green_current_ci_dispatches_zero_repair_codex = function()
    local stale_key = "head:def456/checks:digest-0000000101"
    local event = fixing({
      repair_input = "ci-failure",
      ci_failure_key = stale_key,
      gate_failure_excerpt = "own-ci-red",
      gate_baseline_sha = "abc123",
    })
    event.work_unit_key = payloads_builders.fixing_work_unit_key(event)
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    local gate_marker = m_builders.merge_gate_marker(
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      "own-ci-red",
      nil,
      stale_key
    )
    local comments = {
      core.state_marker(event.proposal_id, "fixing", event.version),
      gate_marker,
    }
    local green_rollup = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"SUCCESS","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"test","headSha":"def456"}]'
    mock_bot_env()
    mock_real_write_env_reads()
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, comments, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456", nil, nil, nil, 4, green_rollup)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456", nil, {
      sha = event.gate_baseline_sha,
      exit_code = 0,
      stdout = "",
      stderr = "",
    })
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, comments, branch, event.version)

    local result
    with_codex_runs({}, function()
      result = run_fix(event, opts("fix-ci-stale-handoff-green", { FKST_GITHUB_WRITE = "1" }))
    end)
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    local reviewing = find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return type(payload.handoff) == "table" and payload.handoff.kind == "github-devloop.reviewing"
    end)
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.handoff.version, core.next_fix_version(event.version))
  end,

  -- LOAD-BEARING REGRESSION: without routing the ci-failure refix through the fix-round cap,
  -- an alternating predecessor set mints fixing versions forever. At max_fix_rounds() the
  -- refix must NOT mint another generation; the single admission owner emits the same
  -- WHY-bearing, head-bound terminal intent as every other own-CI entrance.
  test_ci_failure_speculative_refix_terminates_at_max_fix_rounds = function()
    local ci_failure_key = "head:def456/checks:digest-0000000101"
    local event = fixing_at_round(config.max_fix_rounds(), { ci_failure_key = ci_failure_key })
    t.eq(event.repair_input, "ci-failure")
    t.eq(core.version_fix_round(event.version), config.max_fix_rounds())
    local result = run_speculative_refix(event, speculative_feedback_comments(event, ci_failure_key), "fix-ci-refix-at-cap")
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    t.eq(find_fixing_handoff(result), nil)
    t.eq(find_attempt_fact(result), nil)
    local reconcile = find_raise(result.raises, "devloop_fix_reconcile")
    local decompose = find_raise(result.raises, "github-devloop-decompose.devloop_decompose")
    t.is_true(reconcile ~= nil)
    t.is_true(decompose ~= nil)
    t.eq(reconcile.payload.bound_head_sha, event.reviewed_head_sha)
    t.eq(reconcile.payload.round, config.max_fix_rounds())
    t.eq(reconcile.payload.reason_class, "fix-loop-max-rounds")
  end,

  test_ci_failure_speculative_refix_admits_one_generation_below_cap = function()
    local ci_failure_key = "head:def456/checks:digest-0000000101"
    local event = fixing_at_round(config.max_fix_rounds() - 1, { ci_failure_key = ci_failure_key })
    local result = run_speculative_refix(event, speculative_feedback_comments(event, ci_failure_key), "fix-ci-refix-below-cap")
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    local handoff = find_fixing_handoff(result)
    t.is_true(handoff ~= nil)
    t.eq(handoff.payload.handoff.repair_input, "ci-failure")
    -- exactly one admitted generation, monotonically advancing to the cap
    t.eq(core.version_fix_round(handoff.payload.handoff.version), config.max_fix_rounds())
    t.is_true(handoff.payload.handoff.ci_failure_key ~= ci_failure_key)
    t.is_true(handoff.payload.handoff.ci_failure_key:find("head:def456/checks:", 1, true) == 1)
    t.eq(find_attempt_fact(result), nil)
  end,

  test_review_feedback_speculative_refix_stays_uncapped_over_the_cap = function()
    -- Same speculative predecessor churn but repair_input = review-feedback (a speculative
    -- merge conflict, ci_failure_key nil): the branch is byte-for-byte the prior uncapped mint
    -- (next_fix_version), even well past the own-CI-red cap. Contrast with the ci-failure path
    -- which terminates at the cap.
    local event = fixing_at_round(config.max_fix_rounds() + 2) -- no ci_failure_key => review-feedback
    t.eq(event.repair_input, "review-feedback")
    t.is_true(core.version_fix_round(event.version) > config.max_fix_rounds())
    local result = run_speculative_refix(event, speculative_feedback_comments(event, nil), "fix-review-refix-over-cap")
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    local handoff = find_fixing_handoff(result)
    t.is_true(handoff ~= nil)
    t.eq(handoff.payload.handoff.repair_input, "review-feedback")
    -- uncapped: advances past the cap, no terminal / attempt fact.
    t.eq(core.version_fix_round(handoff.payload.handoff.version), core.version_fix_round(event.version) + 1)
    t.eq(find_attempt_fact(result), nil)
  end,

  test_completed_ci_repair_attempt_defers_duplicate_delivery_without_spawning = function()
    local ci_failure_key = "head:def456/checks:digest-0000000101"
    local event = fixing({
      repair_input = "ci-failure",
      ci_failure_key = ci_failure_key,
      gate_failure_excerpt = "own-ci-red",
    })
    event.work_unit_key = payloads_builders.fixing_work_unit_key(event)
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    local merge_gate_marker = m_builders.merge_gate_marker(event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      nil,
      "own-ci-red",
      nil,
      ci_failure_key
    )
    local attempt_comment = ci_repair_attempts.comment_request("owner/repo", event, "no-fix", "No repaired revision was published.").body
    mock_bot_env()
    mock_real_write_env_reads()
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      merge_gate_marker,
      attempt_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")

    local result = run_fix(event, opts("fix-ci-duplicate-attempt-defers", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,

  test_speculative_refix_replaces_stale_payload_key_with_fresh_verdict_key = function()
    local ci_failure_key = "head:def456/checks:digest-0000000101"
    local event = fixing({
      repair_input = "ci-failure",
      ci_failure_key = ci_failure_key,
      predecessor_set = "none",
      gate_failure_excerpt = "own-ci-red",
    })
    event.work_unit_key = payloads_builders.fixing_work_unit_key(event)
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local merge_ready_version = core._strip_latest_fix_version_suffix(event.version)
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    local merge_gate_marker = m_builders.merge_gate_marker(event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      nil,
      "own-ci-red",
      "none",
      ci_failure_key
    )
    local current_pr_comments = {
      origin_marker,
      core.state_marker(event.proposal_id, "merge-ready", merge_ready_version),
      m_builders.merge_ready_marker(event.proposal_id,
        event.pr_number,
        merge_ready_version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha
      ),
      m_builders.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
      core.state_marker(event.proposal_id, "fixing", event.version),
      merge_gate_marker,
    }
    mock_bot_env()
    mock_real_write_env_reads()
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      merge_gate_marker,
    }, branch, event.version)
    mock_pr_fix(current_pr_comments, branch, "def456")
    h.mock_required_check_runs_for("def456", "failure", "owner/repo")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456")
    mock_merge_queue_list({ 6 })
    mock_merge_queue_pr(6,
      41,
      "ready/consensus-github-devloop/issue/owner/repo/41/2026-06-03T00-00-00Z",
      "abc999"
    )
    local result
    with_codex_runs({}, function()
      result = run_fix(event, opts("fix-ci-refix-preserves-key", { FKST_GITHUB_WRITE = "1" }))
    end)
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return type(payload.handoff) == "table" and payload.handoff.kind == "github-devloop.fixing"
    end)
    t.is_true(comment_raise ~= nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.fixing")
    t.eq(comment_raise.payload.handoff.repair_input, "ci-failure")
    local fresh_key = comment_raise.payload.handoff.ci_failure_key
    t.is_true(fresh_key ~= ci_failure_key)
    t.is_true(fresh_key:find("head:def456/checks:", 1, true) == 1)
    t.is_true(comment_raise.payload.body:find('ci_failure_key="' .. fresh_key .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.dedup_key:find(fresh_key, 1, true) == nil)
  end,

  test_ci_repair_attempt_fact_is_version_keyed_and_returns_comment_epoch = function()
    local event = fixing({
      repair_input = "ci-failure",
      ci_failure_key = "head:def456/checks:digest-0000000101",
    })
    event.work_unit_key = payloads_builders.fixing_work_unit_key(event)
    local request = ci_repair_attempts.comment_request("owner/repo", event, "no-fix", "No repaired revision was published.")
    local created_at = "2026-06-03T01:02:03Z"
    local fact = ci_repair_attempts.fact({ {
      body = request.body,
      author_login = "fkst-test-bot",
      created_at = created_at,
    } }, {
      proposal_id = event.proposal_id,
      pr_number = event.pr_number,
      version = event.version,
      reviewed_head_sha = "feedface",
      ci_failure_key = "head:feedface/checks:digest-0000000202",
      work_unit_key = "changed-work-unit",
    })

    t.is_true(fact ~= nil)
    t.eq(fact.version, event.version)
    t.eq(fact.fix_round, core.version_fix_round(event.version))
    t.eq(fact.comment_created_at, created_at)
    t.is_true(request.body:find('version="' .. event.version .. '"', 1, true) ~= nil)
    t.is_true(request.body:find('work_unit_key="', 1, true) == nil)
  end,

  test_ci_repair_payload_identity_ignores_head_failure_key_and_baseline_churn = function()
    local template = fixing()
    local first = payloads_builders.build_devloop_fixing_payload({
      proposal_id = template.proposal_id,
      impl_version = template.version,
    }, template.pr_number, {
      review_proposal_id = template.review_proposal_id,
      review_dedup_key = template.review_dedup_key,
      reviewed_head_sha = "def456",
      gate_baseline_sha = "abc123",
      ci_failure_key = "head:def456/checks:digest-0000000101",
    }, template.source_ref)
    local second = payloads_builders.build_devloop_fixing_payload({
      proposal_id = first.proposal_id,
      impl_version = first.version,
    }, first.pr_number, {
      review_proposal_id = first.review_proposal_id,
      review_dedup_key = first.review_dedup_key .. "/changed",
      reviewed_head_sha = "feedface",
      gate_baseline_sha = "ba5e9999",
      ci_failure_key = "head:feedface/checks:digest-0000000202",
    }, first.source_ref)

    t.eq(first.work_unit_key, second.work_unit_key)
    t.eq(first.dedup_key, second.dedup_key)
    local first_replay = payloads_builders.build_replayed_fixing_payload({
      proposal_id = first.proposal_id,
      impl_version = first.version,
    }, first.pr_number, first, first.source_ref)
    local second_replay = payloads_builders.build_replayed_fixing_payload({
      proposal_id = second.proposal_id,
      impl_version = second.version,
    }, second.pr_number, second, second.source_ref)
    t.eq(first_replay.dedup_key, second_replay.dedup_key)
  end,
}
