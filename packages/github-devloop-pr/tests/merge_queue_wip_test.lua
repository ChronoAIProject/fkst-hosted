local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local h = require("tests.devloop_helpers")
local transition_version = require("contract.transition_version")
local payloads_builders = require("devloop.payloads.builders")
local m_mq = require("devloop.merge_queue")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
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
local mock_merging_comment = h.mock_merging_comment
local mock_issue_close = h.mock_issue_close
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function json_literal(value)
  return '"' .. json_string(value) .. '"'
end

local function branch_for_pr(pr_number)
  return "devloop-owner-repo-" .. tostring(pr_number)
end

local function run_merge_queue_tick(run_opts)
  return t.run_department("departments/merge_queue/main.lua", {
    queue = "devloop_merge_queue_tick",
    payload = {
      schema = "github-devloop.merge-queue-tick.v1",
    },
  }, run_opts)
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

local function mock_repo_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_config_env(times)
  for _ = 1, times or 1 do
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
  end
end

local function mock_write_env_many(times)
  for _ = 1, times do
    mock_write_env("1")
  end
end

local function merge_comments_with_origin(event, origin_marker)
  local comments = { origin_marker }
  for _, comment in ipairs(merge_comments(event)) do
    table.insert(comments, comment)
  end
  return comments
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
    m_builders.merge_ready_marker(core, event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
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

local function mock_claimed_issue_for_event(event, times)
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  for _ = 1, times or 1 do
    entity_read_mocks.mock_issue_view_selector(t, {
      repo = "owner/repo",
      number = entity.issue_number,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    }, "assignees,author")
  end
end

local function mock_queue_pr(event, created_at, state, state_version, mergeable, merge_state, rollup_state, rollup_conclusion, base_sha)
  local comments = {}
  for _, comment in ipairs(merge_comments_for_event(event)) do
    table.insert(comments, {
      body = comment,
      author_login = "fkst-test-bot",
      created_at = created_at,
    })
  end
  if state ~= nil then
    table.insert(comments, {
      body = core.state_marker(event.proposal_id, state, state_version or event.version),
      author_login = "fkst-test-bot",
      created_at = created_at,
    })
  end
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = "owner/repo",
    number = event.pr_number,
    comments = comments,
    head = branch_for_pr(event.pr_number),
    head_sha = event.reviewed_head_sha,
    base_sha = base_sha or "abc123",
    updated_at = created_at,
    state = "OPEN",
    mergeable = mergeable or "MERGEABLE",
    merge_state = merge_state or "CLEAN",
    status_check_rollup_json = '[{"name":"test","status":' .. json_literal(rollup_state or "COMPLETED") .. ',"conclusion":' .. json_literal(rollup_conclusion or "SUCCESS") .. '}]',
  })
end

local function mock_queue_pr_red(event, created_at, state, state_version)
  mock_queue_pr(event, created_at, state, state_version, "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")
end

local function mock_merge_pr_view(event, state, mergeable, merge_state, rollup_state, rollup_conclusion, base_sha)
  local comments = {}
  for _, comment in ipairs(merge_comments_for_event(event)) do
    table.insert(comments, {
      body = comment,
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T01:00:00Z",
    })
  end
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = "owner/repo",
    number = event.pr_number,
    comments = comments,
    head = branch_for_pr(event.pr_number),
    head_sha = event.reviewed_head_sha,
    base_sha = base_sha or "abc123",
    state = state or "OPEN",
    mergeable = mergeable or "MERGEABLE",
    merge_state = merge_state or "CLEAN",
    status_check_rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":' .. json_literal(rollup_conclusion or "SUCCESS") .. ',"detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":' .. json_literal(rollup_state or "COMPLETED") .. ',"workflowName":"test"}]',
  })
end

local function mock_merged_pr_view(event)
  local entity = entity_lib.parse_entity_proposal_id(event.proposal_id)
  local comments = {
    m_builders.pr_origin_marker(core, event.proposal_id, entity and entity.issue_number or 42, branch_for_pr(event.pr_number), event.version, "dev"),
    core.state_marker(event.proposal_id, "merge-ready", event.version),
    m_builders.merge_ready_marker(core, event.proposal_id, event.pr_number, event.version, event.review_proposal_id, event.review_dedup_key, event.reviewed_head_sha),
    m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "approve", event.review_dedup_key),
  }
  table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
  table.insert(comments, m_builders.merging_marker(core, event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha))
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = "owner/repo",
    number = event.pr_number,
    comments = comments,
    head = branch_for_pr(event.pr_number),
    head_sha = event.reviewed_head_sha,
    state = "MERGED",
    merged_at = "2026-06-03T02:05:04Z",
    status_check_rollup_json = '[{"name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]',
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

local function mock_candidate_head_contains_base(event, contains)
  t.mock_command("git fetch 'origin' '" .. branch_for_pr(event.pr_number) .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify FETCH_HEAD^{commit}", {
    stdout = tostring(event.reviewed_head_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor", {
    stdout = "",
    stderr = "",
    exit_code = contains == false and 1 or 0,
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
  entity_read_mocks.mock_issue_list_raw_command(t, core.gh_issue_list_wip_cmd("owner/repo"), {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
  })
end

local function mock_wip_issue_state(issue_number, state)
  local proposal_id = base_ids.proposal_id("owner/repo", issue_number)
  t.mock_command(core.gh_issue_view_state_cmd("owner/repo", issue_number), {
    stdout = string.format(
      '{"title":"Issue","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      render_comment(core.state_marker(proposal_id, state, "ready/consensus-github-devloop/issue/owner/repo/" .. tostring(issue_number) .. "/2026-06-03T01-02-03Z"))
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function predecessor_set_for(event)
  return "pr" .. tostring(event.pr_number)
    .. "-" .. transition_version.safe_version_segment(event.proposal_id)
    .. "-" .. transition_version.safe_version_segment(event.version)
    .. "-" .. tostring(event.reviewed_head_sha)
end

return {
  test_merge_queue_head_orders_by_trusted_merge_ready_time_then_pr_number = function()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local newer = event_for_pr(7, 42, "2026-06-03T01-02-03Z", "def456")
    mock_bot_env()
    mock_queue_list({ 9, 7 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z")
    mock_queue_pr(newer, "2026-06-03T02:00:00Z")

    local head = m_mq.merge_queue_head(core, "owner/repo", "dev")
    t.eq(head.pr_number, 9)
    t.eq(head.proposal_id, older.proposal_id)

    mock_bot_env()
    local left = event_for_pr(3, 45, "2026-06-03T00-00-00Z", "aaa333")
    local right = event_for_pr(2, 46, "2026-06-03T00-00-00Z", "aaa222")
    mock_queue_list({ 3, 2 })
    mock_queue_pr(left, "2026-06-03T01:00:00Z")
    mock_queue_pr(right, "2026-06-03T01:00:00Z")
    head = m_mq.merge_queue_head(core, "owner/repo", "dev")
    t.eq(head.pr_number, 2)

    mock_bot_env()
    mock_queue_list({ 9, 7 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z", "fixing", older.version .. "/fix/1")
    mock_queue_pr(newer, "2026-06-03T02:00:00Z")
    head = m_mq.merge_queue_head(core, "owner/repo", "dev")
    t.eq(head.pr_number, 7)
    t.eq(head.proposal_id, newer.proposal_id)
  end,

  test_merge_queue_head_treats_missing_marker_time_as_unknown_not_oldest = function()
    local dated = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local undated = event_for_pr(7, 42, "2026-06-03T01-02-03Z", "def456")
    mock_bot_env()
    mock_queue_list({ 7, 9 })
    mock_queue_pr(undated, "")
    mock_queue_pr(dated, "2026-06-03T01:00:00Z")

    local head = m_mq.merge_queue_head(core, "owner/repo", "dev")
    t.eq(head.pr_number, 9)
    t.eq(head.proposal_id, dated.proposal_id)
  end,

  test_merge_non_head_holds_without_merge_side_effects = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
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

  test_fixing_head_yields_merge_queue_lane = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aaa111")
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(current))
    mock_pr_merge({ origin_marker })
    mock_queue_list({ 9 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z", "fixing", older.version .. "/fix/1")

    local result = run_merge(current, opts("merge-queue-fixing-head", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
  end,

  test_merge_queue_poll_drives_current_head_after_non_head_event_held = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aabb11")
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(current))
    mock_pr_merge({ origin_marker })
    mock_queue_list({ 9 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z")

    local held = run_merge(current, opts("merge-queue-poll-held", { FKST_GITHUB_WRITE = "1" }))
    t.eq(held.exit_code, 0)
    t.eq(find_raise(held.raises, "github-proxy.github_pr_comment_request"), nil)

    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7 })
    mock_queue_pr(current, "2026-06-03T02:00:00Z")
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_write_env("1")
    mock_claimed_issue_for_event(current, 2)
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_merging_comment()
    t.mock_command("gh pr merge '7' --repo 'owner/repo' --merge --match-head-commit 'def456'", {
      stdout = "merged\n",
      stderr = "",
      exit_code = 0,
    })
    mock_pr_merge({ origin_marker }, "devloop-owner-repo-42-01HY", "def456", "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_issue_close()
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_queue_list({})

    local polled = run_merge_queue_tick(opts("merge-queue-poll-drives-head", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(find_raise(polled.raises, "github-proxy.github_issue_label_request"), nil)
    t.is_true(find_raise(polled.raises, "github-proxy.github_pr_comment_request").payload.body:find("fkst:github-devloop:merged:v1", 1, true) ~= nil)
  end,

  test_queue_starvation_redrive_processes_current_merge_queue_head = function()
    local current = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_repo_env()
    mock_queue_list({ 7 })
    mock_queue_pr(current, "2026-06-03T02:00:00Z")
    mock_claimed_issue_for_event(current, 1)
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))

    local result = run_starvation_merge_queue_tick(current, opts("merge-queue-starvation-redrive", {
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    local reconcile = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(reconcile.payload.body:find("fkst:github-devloop:queue-starvation-reconcile:v1", 1, true) ~= nil)
    t.is_true(reconcile.payload.body:find('outcome="head-redriven"', 1, true) ~= nil)
  end,

  test_merge_queue_poll_skips_other_owned_head_before_pr_work = function()
    local current = merge_ready()
    mock_bot_env()
    mock_repo_env()
    mock_queue_list({ 7 })
    mock_queue_pr(current, "2026-06-03T02:00:00Z")
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = '{"assignees":[{"login":"human"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge_queue_tick(opts("merge-queue-poll-other-owned", {
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_queue_tick_dispatches_namespaced_queue_to_scan = function()
    -- Regression: the fanout tick is delivered with a NAMESPACED event.queue
    -- ("github-devloop-pr.devloop_merge_queue_tick") in production, while the
    -- department declares the bare name. A bare-only `event.queue ==` compare
    -- dropped the tick into the per-PR merge_ready branch ("unsupported event
    -- payload") so the merge-queue scan never ran, and merge-ready PRs that did
    -- not merge on their first per-PR event were never re-attempted (this was
    -- the long-standing "merge is slow" symptom). The namespaced tick must still
    -- reach the scan, which reads the merge queue.
    mock_bot_env()
    mock_repo_env()
    mock_queue_list({})
    local result = t.run_department("departments/merge_queue/main.lua", {
      queue = "github-devloop-pr.devloop_merge_queue_tick",
      payload = { schema = "github-devloop.merge-queue-tick.v1" },
    }, opts("merge-tick-namespaced-dispatch", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    local scanned_queue = false
    for _, call in ipairs(t.command_calls()) do
      if tostring(call.rendered or ""):find("pulls?state=open&base=dev", 1, true) ~= nil then
        scanned_queue = true
      end
    end
    t.is_true(scanned_queue)
  end,

  test_merge_queue_poll_yields_red_fixing_head_to_next_green = function()
    local current = merge_ready()
    local older = event_for_pr(9, 44, "2026-06-03T00-00-00Z", "aabb11")
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_repo_env()
    mock_branch_config_env(2)
    mock_queue_list({ 9, 7 })
    mock_queue_pr_red(older, "2026-06-03T01:00:00Z")
    mock_queue_pr(current, "2026-06-03T02:00:00Z")
    mock_merge_pr_view(older, "OPEN", "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")
    h.mock_required_check_runs_for(older.reviewed_head_sha, "failure")
    mock_diff_name_only(9, { "packages/older.lua" })
    mock_claimed_issue_for_event(older, 1)
    t.mock_command("git fetch origin 'pull/9/merge'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-parse FETCH_HEAD", {
      stdout = "abc123\n",
      stderr = "",
      exit_code = 0,
    })
    mock_queue_list({})

    local first_poll = run_merge_queue_tick(opts("merge-queue-poll-red-head", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.is_true(find_causal_raise(first_poll, "devloop_fixing") ~= nil)
    t.eq(find_raise(first_poll.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")

    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_branch_config_env()
    mock_queue_list({ 9, 7 })
    mock_queue_pr(older, "2026-06-03T01:00:00Z", "fixing", older.version .. "/fix/1")
    mock_queue_pr(current, "2026-06-03T02:00:00Z")
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_write_env("1")
    mock_claimed_issue_for_event(current, 2)
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_pr_merge(merge_comments_with_origin(current, origin_marker))
    mock_merging_comment()
    t.mock_command("gh pr merge '7' --repo 'owner/repo' --merge --match-head-commit 'def456'", {
      stdout = "merged\n",
      stderr = "",
      exit_code = 0,
    })
    mock_pr_merge({ origin_marker }, "devloop-owner-repo-42-01HY", "def456", "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_issue_close()
    mock_diff_name_only(7, { "packages/current.lua" })
    mock_current_base_head("abc123")

    local second_poll = run_merge_queue_tick(opts("merge-queue-poll-yields-red-head", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(find_raise(second_poll.raises, "github-proxy.github_issue_label_request"), nil)
    t.is_true(find_raise(second_poll.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload and payload.body or ""):find("fkst:github-devloop:merged:v1", 1, true) ~= nil
    end) ~= nil)
  end,

  test_speculative_predecessor_set_survives_landed_predecessor = function()
    local predecessor = event_for_pr(5, 41, "2026-06-03T00-00-00Z", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    local current = event_for_pr(7, 42, "2026-06-03T01-02-03Z", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    current.version = current.version .. "/fix/1/fix/2"
    current.review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", current.pr_number, current.version, current.reviewed_head_sha)
    current.review_dedup_key = "consensus:" .. current.review_proposal_id .. "/review"
    current.dedup_key = "merge-ready/" .. current.proposal_id .. "/" .. current.version
    local fix_version = core._strip_latest_fix_version_suffix(current.version)
    local old_review_version = core._strip_latest_fix_version_suffix(fix_version)
    local old_review_proposal = devloop_base.pr_review_proposal_id("owner/repo", current.pr_number, old_review_version, "cccccccccccccccccccccccccccccccccccccccc")
    local old_review_dedup = "consensus:" .. old_review_proposal .. "/review"
    local predecessor_set = predecessor_set_for(predecessor)
    local comments = merge_comments_for_event(current)
    table.insert(comments, core.state_marker(current.proposal_id, "fixing", fix_version))
    table.insert(comments, m_builders.merge_gate_marker(core, 
      current.proposal_id,
      current.pr_number,
      fix_version,
      old_review_proposal,
      old_review_dedup,
      "cccccccccccccccccccccccccccccccccccccccc",
      "1111111111111111111111111111111111111111",
      "mergeable-conflicting",
      predecessor_set
    ))
    table.insert(comments, m_builders.fix_marker(core, current.proposal_id, old_review_proposal, old_review_dedup, "cccccccccccccccccccccccccccccccccccccccc", current.reviewed_head_sha))
    mock_bot_env()
    mock_write_env("1")
    mock_branch_config_env(4)
    mock_issue_merge({ "fkst-dev:merge-ready" }, comments)
    mock_pr_merge(comments, branch_for_pr(current.pr_number), current.reviewed_head_sha)
    mock_queue_list({})
    mock_current_base_head("dddddddddddddddddddddddddddddddddddddddd")
    t.mock_command("git merge-base --is-ancestor " .. predecessor.reviewed_head_sha .. " dddddddddddddddddddddddddddddddddddddddd", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    mock_write_env("1")
    mock_claimed_issue_for_event(current, 2)
    mock_pr_merge(comments, branch_for_pr(current.pr_number), current.reviewed_head_sha)
    mock_pr_merge(comments, branch_for_pr(current.pr_number), current.reviewed_head_sha)
    mock_current_base_head("dddddddddddddddddddddddddddddddddddddddd")
    t.mock_command("git merge-base --is-ancestor " .. predecessor.reviewed_head_sha .. " dddddddddddddddddddddddddddddddddddddddd", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    mock_merging_comment()
    t.mock_command("gh pr merge '7' --repo 'owner/repo' --merge --match-head-commit '" .. current.reviewed_head_sha .. "'", {
      stdout = "merged\n",
      stderr = "",
      exit_code = 0,
    })
    mock_pr_merge({ m_builders.pr_origin_marker(core, current.proposal_id, "42", branch_for_pr(current.pr_number), current.version, "dev") }, branch_for_pr(current.pr_number), current.reviewed_head_sha, "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_issue_close()

    local result = run_merge(current, opts("merge-speculative-landed-predecessor", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.is_true(find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body:find("fkst:github-devloop:merged:v1", 1, true) ~= nil)
  end,

  test_merge_conflicting_but_current_base_contained_waits_without_fixing = function()
    local current_head = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    local base_event = merge_ready()
    local current_review = devloop_base.pr_review_proposal_id("owner/repo", base_event.pr_number, base_event.version, current_head)
    local current = merge_ready({
      review_proposal_id = current_review,
      review_dedup_key = "consensus:" .. current_review .. "/review",
      reviewed_head_sha = current_head,
    })
    local origin_marker = m_builders.pr_origin_marker(core, current.proposal_id, "42", "devloop-owner-repo-42-01HY", current.version, "dev")
    local base_head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(current))
    mock_pr_merge({ origin_marker }, "devloop-owner-repo-42-01HY", current.reviewed_head_sha, "OPEN", "owner/repo", false, "MERGEABLE", "DIRTY")
    mock_current_base_head(base_head)
    t.mock_command("git merge-base --is-ancestor " .. base_head .. " " .. current.reviewed_head_sha, {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge(current, opts("merge-conflicting-base-contained", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,

  test_merge_batch_window_merges_disjoint_pair_in_one_pass = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env_many(64)
    mock_repo_env()
    mock_queue_list({ 7, 8 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_claimed_issue_for_event(second)
    mock_candidate_head_contains_base(second, true)
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_merge_pr_view(second)
    mock_claimed_issue_for_event(second, 2)
    mock_merge_pr_view(second)
    mock_merge_pr_view(second)
    mock_merge_command(second)
    mock_merged_pr_view(second)
    mock_issue_close_for(second)
    mock_current_base_head("abc125")
    mock_queue_list({})

    local result = run_merge_queue_tick(opts("merge-batch-window-disjoint", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 2)
    t.eq(count_calls("gh issue close"), 0)
    t.eq(#result.raises, 2)
  end,

  test_merge_batch_window_stops_when_candidate_head_lacks_current_base = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env_many(64)
    mock_repo_env()
    mock_queue_list({ 7, 8 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_claimed_issue_for_event(second)
    mock_candidate_head_contains_base(second, false)
    mock_queue_list({ 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")

    local result = run_merge_queue_tick(opts("merge-batch-window-current-base-missing", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 0)
    t.eq(count_calls("gh pr diff '8' --repo 'owner/repo' --name-only"), 0)
    local chained = find_raise(result.raises, "devloop_merge_queue_tick")
    t.is_true(chained ~= nil)
    t.eq(chained.payload.schema, "github-devloop.merge-queue-tick.v1")
    t.eq(chained.payload.cause.merged_pr_number, 7)
    t.eq(chained.payload.cause.next_pr_number, 8)
    t.is_true(chained.payload.dedup_key:find("merged-pr/7/next-pr/8/fed789", 1, true) ~= nil)
  end,

  test_merge_queue_self_requeue_is_quiescent_when_queue_empty_after_progress = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    mock_bot_env()
    mock_write_env_many(64)
    mock_repo_env()
    mock_queue_list({ 7 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_merge_pr_view(first)
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_queue_list({})

    local result = run_merge_queue_tick(opts("merge-queue-self-requeue-empty", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(find_raise(result.raises, "devloop_merge_queue_tick"), nil)
  end,

  test_merge_queue_chained_unknown_mergeability_retries_to_completion = function()
    local next = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env_many(64)
    mock_repo_env()
    mock_queue_list({ 8 })
    mock_queue_pr(next, "2026-06-03T01:01:00Z")
    mock_claimed_issue_for_event(next)
    mock_merge_pr_view(next, "OPEN", "UNKNOWN", "CLEAN")

    local retry = run_merge_queue_tick(opts("merge-queue-self-requeue-unknown", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(retry.exit_code, 1)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(retry.raises, "devloop_fixing"), nil)

    mock_bot_env()
    mock_write_env_many(64)
    mock_repo_env()
    mock_queue_list({ 8 })
    mock_queue_pr(next, "2026-06-03T01:01:00Z")
    mock_claimed_issue_for_event(next)
    mock_merge_pr_view(next)
    mock_merge_pr_view(next)
    mock_merge_pr_view(next)
    mock_merge_command(next)
    mock_merged_pr_view(next)
    mock_issue_close_for(next)
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_current_base_head("abc124")
    mock_queue_list({})

    local completed = run_merge_queue_tick(opts("merge-queue-self-requeue-unknown-retry", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(completed.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(find_raise(completed.raises, "devloop_fixing"), nil)
  end,

  test_merge_batch_window_stops_on_overlapping_files = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7, 8 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/shared.lua" })
    mock_current_base_head("abc124")
    mock_claimed_issue_for_event(second)
    mock_candidate_head_contains_base(second, true)
    mock_diff_name_only(8, { "packages/shared.lua" })
    mock_queue_list({ 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z")

    local result = run_merge_queue_tick(opts("merge-batch-window-overlap", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 0)
    local chained = find_raise(result.raises, "devloop_merge_queue_tick")
    t.is_true(chained ~= nil)
    t.eq(chained.payload.cause.merged_pr_number, 7)
    t.eq(chained.payload.cause.next_pr_number, 8)
  end,

  test_merge_batch_window_stops_when_candidate_gate_fails = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7, 8 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_claimed_issue_for_event(second)
    mock_candidate_head_contains_base(second, true)
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_write_env("1")
    mock_branch_config_env()
    mock_queue_list({})
    mock_claimed_issue_for_event(second, 1)
    mock_merge_pr_view(second, "OPEN", "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")
    h.mock_required_check_runs_for(second.reviewed_head_sha, "failure")
    mock_queue_list({ 8 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z", "fixing", second.version .. "/fix/1")

    local result = run_merge_queue_tick(opts("merge-batch-window-gate-fails", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 0)
    t.eq(find_causal_raise(result, "devloop_fixing") ~= nil, true)
  end,

  test_merge_batch_window_does_not_skip_failed_candidate_to_later_disjoint_pr = function()
    local first = event_for_pr(7, 42, "2026-06-03T00-00-00Z", "def456")
    local second = event_for_pr(8, 43, "2026-06-03T00-01-00Z", "fed789")
    local third = event_for_pr(9, 44, "2026-06-03T00-02-00Z", "abc999")
    mock_bot_env()
    mock_write_env("1")
    mock_repo_env()
    mock_queue_list({ 7, 8, 9 })
    mock_queue_pr(first, "2026-06-03T01:00:00Z")
    mock_queue_pr(second, "2026-06-03T01:01:00Z")
    mock_queue_pr(third, "2026-06-03T01:02:00Z")
    mock_merge_pr_view(first)
    mock_write_env("1")
    mock_claimed_issue_for_event(first, 2)
    mock_merge_pr_view(first)
    mock_merge_pr_view(first)
    mock_merge_command(first)
    mock_merged_pr_view(first)
    mock_issue_close_for(first)
    mock_diff_name_only(7, { "packages/a.lua" })
    mock_current_base_head("abc124")
    mock_claimed_issue_for_event(second)
    mock_candidate_head_contains_base(second, true)
    mock_diff_name_only(8, { "packages/b.lua" })
    mock_write_env("1")
    mock_write_env("1")
    mock_write_env("1")
    mock_branch_config_env()
    mock_queue_list({})
    mock_claimed_issue_for_event(second, 1)
    mock_merge_pr_view(second, "OPEN", "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")
    h.mock_required_check_runs_for(second.reviewed_head_sha, "failure")
    mock_queue_list({ 8, 9 })
    mock_queue_pr(second, "2026-06-03T01:01:00Z", "fixing", second.version .. "/fix/1")
    mock_queue_pr(third, "2026-06-03T01:02:00Z")

    local result = run_merge_queue_tick(opts("merge-batch-window-no-skip-after-gate-fail", {
      FKST_GITHUB_WRITE = "1",
      FKST_GITHUB_REPO = "owner/repo",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(count_calls("gh issue close"), 0)
    t.eq(count_calls("gh pr diff '9' --repo 'owner/repo' --name-only"), 0)
  end,

}
