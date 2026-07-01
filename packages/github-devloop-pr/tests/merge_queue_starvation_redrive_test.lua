local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local contract_time = require("contract.time")
local payloads_builders = require("devloop.payloads.builders")
local m_builders = require("devloop.markers.builders")
local m_mq = require("devloop.merge_queue")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_pr_merge = h.mock_pr_merge
local mock_issue_close = h.mock_issue_close
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
  return t.run_department("departments/merge_queue/main.lua", {
    queue = "devloop_merge_queue_tick",
    payload = m_mq.merge_queue_starvation_tick_payload(core, "owner/repo", "merge-ready/pr/" .. tostring(event.pr_number), {
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
  local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", pr_number, version, head_sha)
  return payloads_builders.build_devloop_merge_ready_payload(core, proposal_id, pr_number, version, {
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
    reviewed_head_sha = head_sha,
  }, {
    kind = "external",
    ref = "owner/repo#pr/" .. tostring(pr_number),
  })
end

local function merge_comments_for_event(event)
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  return {
    m_builders.pr_origin_marker(core, 
      event.proposal_id,
      tostring(entity.issue_number),
      branch_for_pr(event.pr_number),
      event.version,
      "dev"
    ),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    m_builders.merge_ready_marker(core, 
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha
    ),
    m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
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
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", entity.issue_number), {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_command(event)
  t.mock_command("gh pr merge '" .. tostring(event.pr_number) .. "' --repo 'owner/repo' --merge --match-head-commit '" .. tostring(event.reviewed_head_sha) .. "'", {
    stdout = "merged\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_normal_risk_merge_gate(event)
  for _ = 1, 2 do
    t.mock_command("gh pr diff '" .. tostring(event.pr_number) .. "' --repo 'owner/repo' --name-only", {
      stdout = "file.lua\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_merging_comment_for_event(event)
  t.mock_command("gh pr comment '" .. tostring(event.pr_number) .. "' --repo 'owner/repo' --body-file", {
    stdout = "commented\n",
    stderr = "",
    exit_code = 0,
  })
end

local function merged_comments_for_event(event)
  local comments = merge_comments_for_event(event)
  table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
  table.insert(comments, m_builders.merging_marker(core, event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha))
  return comments
end

return {
  test_queue_starvation_scheduler_selects_reported_aged_entry_behind_fifo_head = function()
    local fifo_head = merge_ready()
    local aged = event_for_pr(459, 459, "2026-06-03T00-00-00Z", "abcdef1234567890abcdef1234567890abcdef12")
    local entries = {
      {
        pr_number = fifo_head.pr_number,
        proposal_id = fifo_head.proposal_id,
        version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T02-50-00Z",
        state = "merge-ready",
        head_sha = fifo_head.reviewed_head_sha,
        merge_ready_created_at = "2026-06-03T01:00:00Z",
      },
      {
        pr_number = aged.pr_number,
        proposal_id = aged.proposal_id,
        version = aged.version,
        state = "merge-ready",
        head_sha = aged.reviewed_head_sha,
        merge_ready_created_at = "2026-06-03T02:00:00Z",
      },
    }

    local selected, age = m_mq.merge_queue_starvation_candidate(core, entries, 60, contract_time.iso_timestamp_epoch_seconds("2026-06-03T02:30:00Z"))

    t.eq(selected.pr_number, 459)
    t.eq(selected.proposal_id, aged.proposal_id)
    t.eq(age, 150)
  end,

  test_queue_starvation_redrive_merges_reported_aged_entry_behind_fifo_head = function()
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
    mock_pr_merge(merge_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha)
    mock_pr_merge(merge_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha)
    mock_normal_risk_merge_gate(stale)
    mock_merging_comment_for_event(stale)
    mock_merge_command(stale)
    mock_pr_merge(merged_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha, "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_issue_close()
    mock_queue_list({})

    local result = run_starvation_merge_queue_tick(stale, opts("merge-queue-starvation-non-reported-head", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge '" .. tostring(stale.pr_number) .. "' --repo 'owner/repo' --merge --match-head-commit"), 1)
    local reconcile = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.pr_number, stale.pr_number)
    t.is_true(reconcile.payload.body:find("fkst:github-devloop:queue-starvation-reconcile:v1", 1, true) ~= nil)
    t.is_true(reconcile.payload.body:find('pr="' .. tostring(stale.pr_number) .. '"', 1, true) ~= nil)
    t.is_true(reconcile.payload.body:find('head_sha="' .. stale.reviewed_head_sha .. '"', 1, true) ~= nil)
    t.is_true(find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload and payload.body or ""):find("fkst:github-devloop:merged:v1", 1, true) ~= nil
    end) ~= nil)
  end,

  test_queue_starvation_redrive_requeues_after_non_fifo_target_progress = function()
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
    mock_pr_merge(merge_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha)
    mock_pr_merge(merge_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha)
    mock_normal_risk_merge_gate(stale)
    mock_merging_comment_for_event(stale)
    mock_merge_command(stale)
    mock_pr_merge(merged_comments_for_event(stale), branch_for_pr(stale.pr_number), stale.reviewed_head_sha, "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_issue_close()
    mock_queue_list({ 7 })
    mock_queue_pr(current, "2026-06-03T01:00:00Z")

    local result = run_starvation_merge_queue_tick(stale, opts("merge-queue-starvation-requeues-after-progress", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge '" .. tostring(stale.pr_number) .. "' --repo 'owner/repo' --merge --match-head-commit"), 1)
    local chained = find_raise(result.raises, "devloop_merge_queue_tick")
    t.is_true(chained ~= nil)
    t.eq(chained.payload.schema, "github-devloop.merge-queue-tick.v1")
    t.eq(chained.payload.cause.merged_pr_number, stale.pr_number)
    t.eq(chained.payload.cause.next_pr_number, current.pr_number)
  end,
}
