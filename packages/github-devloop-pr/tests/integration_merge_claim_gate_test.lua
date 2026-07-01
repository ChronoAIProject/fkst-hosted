local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_pr_merge = h.mock_pr_merge
local mock_merging_comment = h.mock_merging_comment
local mock_issue_close = h.mock_issue_close
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local json_string = h.json_string
local render_comment = h.render_comment
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local function branch_for_pr(pr_number)
  return "devloop-owner-repo-" .. tostring(pr_number)
end

local function run_direct_merge_ready(payload, run_opts)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  return t.run_department("departments/merge/main.lua", {
    queue = "devloop_merge_ready",
    payload = payload,
  }, run_opts)
end

local function run_merge_queue_tick(run_opts)
  return t.run_department("departments/merge_queue/main.lua", {
    queue = "devloop_merge_queue_tick",
    payload = {
      schema = "github-devloop.merge-queue-tick.v1",
    },
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

local function comments_for(event, created_at, state, state_version)
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  local comments = {
    m_builders.pr_origin_marker(core, event.proposal_id, entity and entity.issue_number or 42, branch_for_pr(event.pr_number), event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    m_builders.merge_ready_marker(core, event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
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

local function mock_repo_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_queue_list(pr_numbers)
  local items = {}
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(items, string.format('{"number":%d,"state":"open","base":{"ref":"dev"},"head":{"ref":"%s","sha":"%s"}}',
      number,
      branch_for_pr(number),
      number == 8 and "fed789" or "def456"
    ))
  end
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&base=dev&per_page=100'", {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_queue_pr(event, created_at, state, state_version, base_sha)
  entity_read_mocks.mock_pr_view_raw_selector(t, { number = event.pr_number }, entity_read_mocks.pr_merge_selector, {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"%s","state":"OPEN","updatedAt":"%s","isDraft":false,"mergedAt":"","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      json_string(branch_for_pr(event.pr_number)),
      json_string(event.reviewed_head_sha),
      json_string(base_sha or "abc123"),
      json_string(created_at),
      comments_for(event, created_at, state, state_version)
    ),
  })
end

local function mock_merge_pr_view(event)
  mock_queue_pr(event, "2026-06-03T01:00:00Z")
end

local function mock_merged_pr_view(event)
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  local comments = {
    m_builders.pr_origin_marker(core, event.proposal_id, entity and entity.issue_number or 42, branch_for_pr(event.pr_number), event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    m_builders.merge_ready_marker(core, event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
    core.state_marker(event.proposal_id, "merging", event.version),
    m_builders.merging_marker(core, event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha),
  }
  local rendered = {}
  for _, comment in ipairs(comments) do
    table.insert(rendered, render_comment(comment))
  end
  entity_read_mocks.mock_pr_view_raw_selector(t, { number = event.pr_number }, entity_read_mocks.pr_merge_selector, {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"dev","baseRefOid":"abc123","state":"MERGED","updatedAt":"2026-06-03T02:03:04Z","isDraft":false,"mergedAt":"2026-06-03T02:05:04Z","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      json_string(branch_for_pr(event.pr_number)),
      json_string(event.reviewed_head_sha),
      table.concat(rendered, ",")
    ),
  })
end

local function mock_issue_claim(issue_number, assignees, author_login)
  local rendered = {}
  for _, assignee in ipairs(assignees or {}) do
    table.insert(rendered, string.format('{"login":"%s"}', json_string(assignee)))
  end
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", issue_number), {
    stdout = string.format(
      '{"assignees":[%s],"author":{"login":"%s"}}\n',
      table.concat(rendered, ","),
      json_string(author_login or "fkst-test-bot")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_claim_failure(issue_number)
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", issue_number), {
    stdout = "",
    stderr = "claim read failed",
    exit_code = 1,
  })
end

local function mock_diff_name_only(pr_number, paths)
  for _ = 1, 3 do
    t.mock_command("gh pr diff '" .. tostring(pr_number) .. "' --repo 'owner/repo' --name-only", {
      stdout = table.concat(paths or {}, "\n") .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_current_base_head(base_sha)
  t.mock_command("git fetch origin dev", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", {
    stdout = tostring(base_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_candidate_head_contains_base(event)
  t.mock_command("git fetch 'origin' '" .. branch_for_pr(event.pr_number) .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'FETCH_HEAD^{commit}'", {
    stdout = tostring(event.reviewed_head_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor", {
    stdout = "",
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

return {
  test_merge_direct_merge_ready_skips_other_owned_issue_before_pr_work = function()
    local current = merge_ready()
    mock_bot_env()
    mock_write_env("1")
    mock_issue_claim(42, { "human" })

    local result = run_direct_merge_ready(current, opts("merge-direct-other-owned", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_direct_merge_ready_fails_closed_when_claim_read_fails = function()
    local current = merge_ready()
    mock_bot_env()
    mock_write_env("1")
    mock_issue_claim_failure(42)

    local result = run_direct_merge_ready(current, opts("merge-direct-claim-fails", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_direct_merge_ready_accepts_unassigned_self_authored_issue = function()
    local current = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_claim(42, {}, "fkst-test-bot")
    mock_pr_merge({ origin_marker })
    mock_queue_list({ 7 })
    mock_queue_pr(current, "2026-06-03T02:00:00Z")

    local result = run_direct_merge_ready(current, opts("merge-direct-unassigned-self-author", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_direct_merge_ready_skips_pr_native_without_backing_issue = function()
    local current = merge_ready({
      proposal_id = entity_lib.pr_proposal_id("owner/repo", 7),
    })
    mock_bot_env()
    mock_write_env("1")

    local result = run_direct_merge_ready(current, opts("merge-direct-no-backing-issue", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_batch_window_stops_before_other_owned_second_entry = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7, 8 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_issue_claim(42, { "fkst-test-bot" })
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close()
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_issue_claim(43, { "human" })
    mock_queue_list({ 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")

    local result = run_merge_queue_tick(opts("merge-batch-window-other-owned-second", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh pr merge '8' --repo 'owner/repo' --merge --match-head-commit 'fed789'"), 0)
    t.eq(count_calls("gh issue close"), 0)
    t.eq(count_calls("git fetch 'origin' '" .. branch_for_pr(8) .. "'"), 0)
    t.eq(count_calls("gh pr diff '8' --repo 'owner/repo' --name-only"), 0)
  end,
}
