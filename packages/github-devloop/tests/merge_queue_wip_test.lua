local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local ready = h.ready
local run_merge = h.run_merge
local run_implement = h.run_implement
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_issue_implement = h.mock_issue_implement
local mock_pr_merge = h.mock_pr_merge
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function branch_for_pr(pr_number)
  return "devloop-owner-repo-" .. tostring(pr_number)
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
  local entity = core.parse_entity_proposal_id(event.proposal_id)
  local comments = {
    core.pr_origin_marker(event.proposal_id, entity and entity.issue_number or 42, branch_for_pr(event.pr_number), event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    core.merge_ready_marker(event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    core.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
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

local function mock_queue_pr(event, created_at, state, state_version, mergeable, merge_state, rollup_state, rollup_conclusion, base_sha)
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

local function mock_merge_pr_view(event, state, mergeable, merge_state, rollup_state, rollup_conclusion, base_sha)
  t.mock_command("--json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"%s","state":"%s","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"mergedAt":"","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"%s","detailsUrl":"https://example.invalid/checks/ci","name":"ci","startedAt":"2026-06-03T02:03:04Z","status":"%s","workflowName":"ci"}]}\n',
      json_string(branch_for_pr(event.pr_number)),
      json_string(event.reviewed_head_sha),
      json_string(base_sha or "abc123"),
      json_string(state or "OPEN"),
      comments_for(event, "2026-06-03T01:00:00Z"),
      json_string(mergeable or "MERGEABLE"),
      json_string(merge_state or "CLEAN"),
      json_string(rollup_conclusion or "SUCCESS"),
      json_string(rollup_state or "COMPLETED")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merged_pr_view(event)
  local entity = core.parse_entity_proposal_id(event.proposal_id)
  local comments = {
    core.pr_origin_marker(event.proposal_id, entity and entity.issue_number or 42, branch_for_pr(event.pr_number), event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    core.merge_ready_marker(event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    core.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
  table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
  table.insert(comments, core.merging_marker(event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha))
  local rendered = {}
  for _, comment in ipairs(comments) do
    table.insert(rendered, render_comment(comment))
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"abc123","state":"MERGED","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"mergedAt":"2026-06-03T02:05:04Z","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      json_string(branch_for_pr(event.pr_number)),
      json_string(event.reviewed_head_sha),
      table.concat(rendered, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_diff_name_only(pr_number, paths)
  t.mock_command("gh pr diff '" .. tostring(pr_number) .. "' --repo 'owner/repo' --name-only", {
    stdout = table.concat(paths or {}, "\n") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_command(event)
  t.mock_command("gh pr comment '" .. tostring(event.pr_number) .. "' --repo 'owner/repo' --body-file", {
    stdout = "commented\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr merge '" .. tostring(event.pr_number) .. "' --repo 'owner/repo' --merge --match-head-commit '" .. tostring(event.reviewed_head_sha) .. "'", {
    stdout = "merged\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_close_for(_event)
  t.mock_command("gh issue close", {
    stdout = "closed\n",
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

  test_merge_batch_window_merges_disjoint_pair_in_one_pass = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_queue_list({ 7, 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_merge_pr_view(second)
    mock_write_env("1")
    mock_write_env("1")
    mock_write_env("1")
    mock_merge_pr_view(second)
    mock_merge_pr_view(second)
    mock_merge_command(second)
    mock_merged_pr_view(second)
    mock_issue_close_for(second)

    local result = run_merge(first, opts("merge-batch-window-disjoint", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 2)
    t.eq(count_calls("gh issue close"), 2)
    t.eq(#result.raises, 4)
  end,

  test_merge_batch_window_stops_on_overlapping_files = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_queue_list({ 7, 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/shared.lua" })
    mock_diff_name_only(8, { "packages/shared.lua" })

    local result = run_merge(first, opts("merge-batch-window-overlap", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 1)
    t.eq(#result.raises, 2)
  end,

  test_merge_batch_window_stops_when_candidate_gate_fails = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_queue_list({ 7, 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_write_env("1")
    mock_write_env("1")
    mock_write_env("1")
    mock_merge_pr_view(second, "OPEN", "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")

    local result = run_merge(first, opts("merge-batch-window-gate-fails", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 1)
    t.eq(find_raise(result.raises, "devloop_fixing") ~= nil, true)
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
