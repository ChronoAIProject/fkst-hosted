local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local payloads_builders = require("devloop.payloads.builders")
local ci_repair_attempts = require("departments.fix.ci_repair_attempts")
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

local function dedup_safe_ci_failure_key(ci_failure_key)
  return base_ids.dedup_key({ tostring(ci_failure_key or "noci") })
end

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

return {
  test_completed_ci_repair_attempt_blocks_duplicate_delivery_without_spawning = function()
    local ci_failure_key = "head:def456/checks:digest-0000000101"
    local event = fixing({
      repair_input = "ci-failure",
      ci_failure_key = ci_failure_key,
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

    local result = run_fix(event, opts("fix-ci-duplicate-attempt-blocks", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex"), 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise ~= nil)
    t.is_true(comment_raise.payload.body:find("own-ci-red-unrepaired", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("attempt-already-completed", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:blocked")
  end,

  test_speculative_refix_preserves_ci_failure_key = function()
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
    t.eq(comment_raise.payload.handoff.ci_failure_key, ci_failure_key)
    t.is_true(comment_raise.payload.body:find('ci_failure_key="' .. ci_failure_key .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.dedup_key:find("/" .. dedup_safe_ci_failure_key(ci_failure_key) .. "/", 1, true) ~= nil)
  end,
}
