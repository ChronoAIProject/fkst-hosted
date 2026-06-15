local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_pr_merge = h.mock_pr_merge
local count_calls = h.count_calls
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function branch_for_pr(pr_number)
  return "devloop-owner-repo-" .. tostring(pr_number)
end

local function mock_repo_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
end

local function run_starvation_merge_queue_tick(event, run_opts)
  return t.run_department("departments/merge/main.lua", {
    queue = "devloop_merge_queue_tick",
    payload = core.merge_queue_starvation_tick_payload("owner/repo", "merge-ready/pr/" .. tostring(event.pr_number), {
      pr_number = event.pr_number,
      proposal_id = event.proposal_id,
      version = event.version,
      head_sha = event.reviewed_head_sha,
    }),
  }, run_opts)
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

local function merge_comments_for_event(event)
  local entity = core.parse_entity_proposal_id(event.proposal_id)
  return {
    core.pr_origin_marker(
      event.proposal_id,
      tostring(entity.issue_number),
      branch_for_pr(event.pr_number),
      event.version,
      "dev"
    ),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    core.merge_ready_marker(
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha
    ),
    core.review_result_marker(event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
end

local function mock_queue_list(pr_numbers)
  local items = {}
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(items, string.format(
      '{"number":%d,"state":"open","base":{"ref":"dev"},"head":{"ref":"%s","sha":"def%d"}}',
      number,
      branch_for_pr(number),
      number
    ))
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=dev&per_page=100'", {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_queue_pr(event, created_at)
  local rendered = {}
  for _, comment in ipairs(merge_comments_for_event(event)) do
    table.insert(rendered, render_comment({
      body = comment,
      author_login = "fkst-test-bot",
      created_at = created_at,
    }))
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"abc123","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"mergedAt":"","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      json_string(branch_for_pr(event.pr_number)),
      json_string(event.reviewed_head_sha),
      table.concat(rendered, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_claimed_issue_for_event(event)
  local entity = core.parse_entity_proposal_id(event.proposal_id)
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", entity.issue_number), {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_queue_starvation_redrive_targets_reported_aged_entry_behind_fifo_head = function()
    local current = merge_ready()
    local stale = event_for_pr(459, 459, "2026-06-03T00-00-00Z", "abcdef1234567890abcdef1234567890abcdef12")
    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7, 459 })
    mock_queue_pr(current, "2026-06-03T01:00:00Z")
    mock_queue_pr(stale, "2026-06-03T02:00:00Z")
    mock_claimed_issue_for_event(stale)
    mock_pr_merge(merge_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha)

    local result = run_starvation_merge_queue_tick(stale, opts("merge-queue-starvation-non-reported-head", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    local reconcile = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.pr_number, stale.pr_number)
    t.is_true(reconcile.payload.body:find("fkst:github-devloop:queue-starvation-reconcile:v1", 1, true) ~= nil)
    t.is_true(reconcile.payload.body:find('pr="' .. tostring(stale.pr_number) .. '"', 1, true) ~= nil)
    t.is_true(reconcile.payload.body:find('head_sha="' .. stale.reviewed_head_sha .. '"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,
}
