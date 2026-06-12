local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local ready = h.ready
local run_merge = h.run_merge
local run_fix = h.run_fix
local run_implement = h.run_implement
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_issue_implement = h.mock_issue_implement
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_merge = h.mock_pr_merge
local mock_pr_fix = h.mock_pr_fix
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function fix_event_for_pr(pr_number, issue_number, version_time, head_sha, predecessor_set)
  local version = "ready/consensus-github-devloop/issue/owner/repo/" .. tostring(issue_number) .. "/" .. tostring(version_time) .. "/fix/1"
  local proposal_id = "github-devloop/issue/owner/repo/" .. tostring(issue_number)
  local review_version = version:gsub("/fix/1$", "")
  local review_proposal_id = core.pr_review_proposal_id("owner/repo", pr_number, review_version, head_sha)
  local payload = {
    schema = "github-devloop.fixing.v1",
    proposal_id = proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
    reviewed_head_sha = head_sha,
    blocking_gap = "merge gate conflict",
    gate_baseline_sha = "abc123",
    gate_failure_excerpt = "mergeable-conflicting",
    predecessor_set = predecessor_set,
    dedup_key = "fixing/" .. tostring(proposal_id) .. "/" .. tostring(pr_number) .. "/" .. tostring(version_time),
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/" .. tostring(pr_number),
    },
  }
  return payload
end

local function merge_gate_comment_for_fix(event, reason)
  return "github-devloop merge gate failed: " .. tostring(reason or "mergeable-conflicting")
    .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
    .. "\n" .. core.merge_gate_marker(
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      reason or "mergeable-conflicting",
      event.predecessor_set
    )
end

local function event_for_pr(pr_number, issue_number, version_time, head_sha)
  local version = "ready/consensus-github-devloop/issue/owner/repo/" .. tostring(issue_number) .. "/" .. tostring(version_time)
  local proposal_id = "github-devloop/issue/owner/repo/" .. tostring(issue_number)
  local review_proposal_id = core.pr_review_proposal_id("owner/repo", pr_number, version, head_sha)
  return core.build_devloop_merge_ready_payload(proposal_id, pr_number, version, {
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
    reviewed_head_sha = head_sha,
  }, {
    kind = "external",
    ref = "owner/repo#pr/" .. tostring(pr_number),
  })
end

local function comments_for(event, created_at, state, state_version)
  local comments = merge_comments(event, "devloop-owner-repo-" .. tostring(event.pr_number), event.version)
  if state ~= nil then
    table.insert(comments, core.state_marker(event.proposal_id, state, state_version or event.version))
  end
  local rendered = {}
  for _, comment in ipairs(comments) do
    table.insert(rendered, render_comment({
      body = comment,
      author_login = "fkst-test-bot",
      created_at = created_at,
    }))
  end
  return table.concat(rendered, ",")
end

local function mock_queue_pr(event, created_at, state, state_version)
  t.mock_command("--json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"devloop-owner-repo-%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"abc123","state":"OPEN","updatedAt":"%s","isDraft":false,"mergedAt":"","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      json_string(event.pr_number),
      json_string(event.reviewed_head_sha),
      json_string(created_at),
      comments_for(event, created_at, state, state_version)
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_queue_list(pr_numbers)
  local items = {}
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(items, string.format('{"number":%d,"state":"open","base":{"ref":"dev"},"head":{"ref":"devloop-owner-repo-%d","sha":"def%d"}}', number, number, number))
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=dev&per_page=100'", {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_wip_issue_list(numbers)
  local items = {}
  for _, number in ipairs(numbers or {}) do
    table.insert(items, string.format('{"number":%d}', number))
  end
  t.mock_command("--state open --label 'fkst-dev:enabled' --limit 100 --json number", {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_wip_issue_state(issue_number, state)
  local proposal_id = core.proposal_id("owner/repo", issue_number)
  t.mock_command("gh issue view '" .. tostring(issue_number) .. "' --repo 'owner/repo' --json labels,state,comments", {
    stdout = string.format(
      '{"state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[%s]}\n',
      render_comment(core.state_marker(proposal_id, state, "ready/consensus-github-devloop/issue/owner/repo/" .. tostring(issue_number) .. "/2026-06-03T01-02-03Z"))
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_pr_view(event, branch, head_sha, mergeable, merge_state)
  local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
  mock_pr_merge(merge_comments(event, branch, event.version, { origin_marker }), branch, head_sha, "OPEN", nil, nil, mergeable, merge_state)
end

return {
  test_merge_queue_head_orders_by_trusted_merge_ready_time_then_pr_number = function()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local newer = event_for_pr(7, 42, "2026-06-03T01-02-03Z", "def456")
    mock_bot_env()
    mock_queue_list({ 9, 7 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z")
    mock_queue_pr(newer, "2026-06-03T02:00:00Z")

    local head = core.merge_queue_head("owner/repo", "dev")
    t.eq(head.pr_number, 9)
    t.eq(head.proposal_id, older.proposal_id)

    mock_bot_env()
    local left = event_for_pr(3, 45, "2026-06-03T00-00-00Z", "aaa333")
    local right = event_for_pr(2, 46, "2026-06-03T00-00-00Z", "aaa222")
    mock_queue_list({ 3, 2 })
    mock_queue_pr(left, "2026-06-03T01:00:00Z")
    mock_queue_pr(right, "2026-06-03T01:00:00Z")
    head = core.merge_queue_head("owner/repo", "dev")
    t.eq(head.pr_number, 2)
  end,

  test_merge_non_head_holds_without_merge_side_effects = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local origin_marker = core.pr_origin_marker(current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(current))
    mock_pr_merge({ origin_marker })
    mock_queue_list({ 9 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z")

    local result = run_merge(current, opts("merge-queue-non-head", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
  end,

  test_fixing_head_still_occupies_merge_queue_lane = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local origin_marker = core.pr_origin_marker(current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(current))
    mock_pr_merge({ origin_marker })
    mock_queue_list({ 9 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z", "fixing", older.version .. "/fix/1")

    local result = run_merge(current, opts("merge-queue-fixing-head", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
  end,

  test_non_head_conflicting_pr_enters_speculative_fixing = function()
    local older = event_for_pr(5, 41, "2026-06-03T00-00-00Z", "aaa111")
    local current = merge_ready({
      pr_number = 7,
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    })
    local branch = core.implement_branch("owner/repo", "42", current.version)
    mock_bot_env()
    mock_write_env("1")
    mock_merge_pr_view(current, branch, "def456", "CONFLICTING", "DIRTY")
    mock_queue_list({ 5 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z")
    mock_write_env("1")

    local result = run_merge(current, opts("merge-queue-speculative-fix", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.is_true(fixing_raise ~= nil)
    t.eq(
      fixing_raise.payload.predecessor_set,
      "pr5-" .. core.safe_version_segment(older.proposal_id) .. "-" .. core.safe_version_segment(older.version) .. "-" .. older.reviewed_head_sha
    )
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_speculative_fix_rederives_when_predecessor_set_changed = function()
    local event = fix_event_for_pr(7, 42, "2026-06-03T01-02-03Z", "def456", "stale-pr5-old-aaa111")
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    local feedback = merge_gate_comment_for_fix(event, "mergeable-conflicting")
    local current_predecessor = event_for_pr(6, 43, "2026-06-03T00-30-00Z", "bbb222")
    local current_merge_ready = event_for_pr(7, 42, "2026-06-03T01-02-03Z", "def456")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
    }, branch, event.version)
    mock_pr_fix({ origin_marker, feedback }, branch, "def456")
    mock_queue_list({ 6, 7 })
    mock_queue_pr(current_predecessor, "2026-06-03T01:30:00Z")
    mock_queue_pr(current_merge_ready, "2026-06-03T02:00:00Z", "fixing", event.version)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456")

    local result = run_fix(event, opts("fix-speculative-predecessor-rederive", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git push origin"), 0)
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.is_true(fixing_raise ~= nil)
    t.eq(fixing_raise.payload.version, core.next_fix_version(event.version))
    t.eq(
      fixing_raise.payload.predecessor_set,
      "pr6-" .. core.safe_version_segment(current_predecessor.proposal_id) .. "-" .. core.safe_version_segment(current_predecessor.version) .. "-" .. current_predecessor.reviewed_head_sha
    )
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise ~= nil)
    t.is_true(comment_raise.payload.body:find('state="fixing" version="' .. core.next_fix_version(event.version) .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('predecessor_set="' .. fixing_raise.payload.predecessor_set .. '"', 1, true) ~= nil)
  end,

  test_wip_cap_blocks_new_implementation_before_codex = function()
    local event = ready()
    mock_bot_env()
    t.mock_command('printf %s "$FKST_DEVLOOP_MAX_INFLIGHT"', {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
    mock_issue_implement({ "fkst-dev:ready" }, { core.state_marker(event.proposal_id, "ready", event.dedup_key) })
    mock_wip_issue_list({ 42, 51 })
    mock_wip_issue_state(51, "implementing")

    local result = run_implement(event, opts("implement-wip-cap", { FKST_DEVLOOP_MAX_INFLIGHT = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(#result.raises, 0)
  end,

  test_wip_cap_allows_start_when_below_capacity = function()
    local event = ready()
    mock_bot_env()
    t.mock_command('printf %s "$FKST_DEVLOOP_MAX_INFLIGHT"', {
      stdout = "2",
      stderr = "",
      exit_code = 0,
    })
    mock_issue_implement({ "fkst-dev:ready" }, { core.state_marker(event.proposal_id, "ready", event.dedup_key) })
    mock_wip_issue_list({ 42, 51 })
    mock_wip_issue_state(51, "ready")
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented")
    mock_git_status("")
    t.mock_command("rev-list --count", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-parse --verify refs/heads", {
      stdout = "def456\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_implement(event, opts("implement-wip-available", { FKST_DEVLOOP_MAX_INFLIGHT = "2" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 1)
    t.is_true(find_raise(result.raises, "devloop_open_pr") ~= nil)
  end,
}
