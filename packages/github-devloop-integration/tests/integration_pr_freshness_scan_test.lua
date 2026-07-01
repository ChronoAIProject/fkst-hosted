local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local branch = "devloop/issue/owner/repo/42/ready-1234567890"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch_sha = "bbbb2222"
local integration_sha = "aaaa1111"
local merge_sha = "cccc3333"
local review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, version, branch_sha)
local review_dedup = "consensus:" .. review_proposal .. "/review"

local function opts(name, extra)
  local env = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
    FKST_DEVLOOP_ROLLUP_MERGE = "auto",
    FKST_GITHUB_WRITE = "",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return { env = env }
end

local function mock_env(write_mode, integration)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = write_mode or "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "auto", stderr = "", exit_code = 0 })
end

local function run_scan(run_opts)
  return t.run_department("departments/pr_freshness_scan/main.lua", {
    queue = "devloop_branch_tick",
    payload = { schema = "github-devloop.branch-tick.v1" },
  }, run_opts or opts("pr-freshness-scan"))
end

local function encode_json_string(value)
  return h.encode_json_string(value)
end

local function render_comments(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, h.render_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function mock_pr_list(is_draft)
  t.mock_command("repos/owner/repo/pulls?state=open", {
    stdout = string.format(
      '[[{"number":7,"headRefOid":"%s","headRefName":"%s","baseRefName":"integration/dev","state":"OPEN","isDraft":%s}]]\n',
      branch_sha,
      encode_json_string(branch),
      is_draft and "true" or "false"
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function pr_comments(state)
  return {
    m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", 42, branch, version, "integration/dev"),
    core.state_marker("github-devloop/issue/owner/repo/42", state or "merge-ready", version),
    m_builders.review_result_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", "approve", review_dedup),
    m_builders.merge_ready_marker(core, "github-devloop/issue/owner/repo/42", 7, version, review_proposal, review_dedup, branch_sha),
  }
end

local function mock_pr_view(state, comments, extra)
  local fields = extra or {}
  local head_repo = fields.head_repo or "owner/repo"
  t.mock_command("gh pr view '7'", {
    stdout = string.format(
      '{"headRefName":"%s","headRefOid":"%s","baseRefName":"integration/dev","state":"%s","updatedAt":"2026-06-03T02:03:04Z","isDraft":%s,"headRepository":{"nameWithOwner":"%s"},"isCrossRepository":%s,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}],"labels":[],"comments":[%s]}\n',
      encode_json_string(fields.head or branch),
      encode_json_string(fields.head_sha or branch_sha),
      encode_json_string(fields.state or "OPEN"),
      fields.is_draft and "true" or "false",
      encode_json_string(head_repo),
      fields.cross_repo and "true" or "false",
      encode_json_string(fields.mergeable or "MERGEABLE"),
      encode_json_string(fields.merge_state_status or "CLEAN"),
      render_comments(comments or pr_comments(state))
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view(labels, comments)
  entity_read_mocks.mock_issue_view_selector(t, {
    labels = labels,
    comments = comments,
  }, "labels,comments")
  entity_read_mocks.mock_issue_view_selector(t, {}, "assignees,author")
end

local function mock_issue_view_other_owned()
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "labels,comments", {
    stdout = '{"labels":[],"comments":[]}\n',
  })
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "assignees,author", {
    stdout = '{"assignees":[{"login":"human"}],"author":{"login":"fkst-test-bot"}}\n',
  })
end

local function mock_fetch_and_heads(current_branch_sha)
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' '" .. branch .. "'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = integration_sha .. "\n", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'" .. branch .. "'^{commit}", { stdout = (current_branch_sha or branch_sha) .. "\n", stderr = "", exit_code = 0 })
end

local function mock_worktree_merge(exit_code, unmerged_stdout)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-rt", stderr = "", exit_code = 0 })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --no-ff --no-commit", { stdout = "", stderr = exit_code == 0 and "" or "conflict", exit_code = exit_code })
  if exit_code ~= 0 then
    t.mock_command("ls-files -u", { stdout = unmerged_stdout or "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_pr_freshness_approved_pr_merges_integration_and_pushes_with_lease = function()
    mock_env("1")
    mock_pr_list(false)
    mock_pr_view("merge-ready")
    mock_issue_view({})
    mock_fetch_and_heads()
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_worktree_merge(0)
    t.mock_command("commit -F", { stdout = "[detached " .. merge_sha .. "] Refresh branch\n", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' '" .. branch .. "'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'" .. branch .. "'^{commit}", { stdout = branch_sha .. "\n", stderr = "", exit_code = 0 })
    t.mock_command("rev-parse HEAD", { stdout = merge_sha .. "\n", stderr = "", exit_code = 0 })
    t.mock_command("--force-with-lease=refs/heads/" .. branch .. ":" .. branch_sha, { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' '" .. branch .. "'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'" .. branch .. "'^{commit}", { stdout = merge_sha .. "\n", stderr = "", exit_code = 0 })

    local result = run_scan(opts("pr-freshness-real", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("merge --no-ff --no-commit"), 1)
    t.eq(h.count_calls("--force-with-lease=refs/heads/" .. branch .. ":" .. branch_sha), 1)
  end,

  test_pr_freshness_skips_arbitrating_fixing_state = function()
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("fixing", {
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", 42, branch, version, "integration/dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", version),
      m_builders.review_result_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", "approve", review_dedup),
    })
    mock_issue_view({})

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("git fetch"), 0)
  end,

  test_pr_freshness_skips_stale_review_approval_for_new_head = function()
    local old_head_sha = branch_sha
    local new_head_sha = "dddd4444"
    local old_review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, version, old_head_sha)
    local old_review_dedup = "consensus:" .. old_review_proposal .. "/review"
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("reviewing", {
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", 42, branch, version, "integration/dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", version .. "/fix/1"),
      m_builders.review_result_marker(core, old_review_proposal, "github-devloop/issue/owner/repo/42", "approve", old_review_dedup),
    }, { head_sha = new_head_sha })
    mock_issue_view({})

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("git fetch"), 0)
  end,

  test_pr_freshness_conflict_raises_sync_conflict_for_pr_branch = function()
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("merge-ready")
    mock_issue_view({})
    mock_fetch_and_heads()
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_worktree_merge(1)

    local result = run_scan()
    t.eq(result.exit_code, 0)
    local raised = h.find_raise(result.raises, "devloop_sync_conflict")
    t.eq(raised.payload.repo, "owner/repo")
    t.eq(raised.payload.upstream_branch, "integration/dev")
    t.eq(raised.payload.integration_branch, branch)
    t.eq(raised.payload.upstream_sha, integration_sha)
    t.eq(raised.payload.integration_sha, branch_sha)
    t.eq(raised.payload.dedup_key, core.pr_freshness_dedup_key("owner/repo", branch, integration_sha))
    t.eq(core.is_supported_sync_conflict(raised.payload), true)
  end,

  test_pr_freshness_skips_other_owned_pr_before_branch_work = function()
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("merge-ready")
    mock_issue_view_other_owned()

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("merge-base --is-ancestor"), 0)
    t.eq(h.count_calls("git fetch"), 0)
  end,

  test_pr_freshness_dry_run_does_not_consume_same_baseline_retry = function()
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("merge-ready")
    mock_issue_view({})
    mock_fetch_and_heads()
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_worktree_merge(0)
    t.mock_command("commit -F", { stdout = "[detached " .. merge_sha .. "] Refresh branch\n", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })

    local run_opts = opts("pr-freshness-once")
    local first = run_scan(run_opts)
    t.eq(first.exit_code, 0)
    mock_env("")
    mock_pr_list(false)
    mock_pr_view("merge-ready")
    mock_issue_view({})
    mock_fetch_and_heads()
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_worktree_merge(0)
    t.mock_command("commit -F", { stdout = "[detached " .. merge_sha .. "] Refresh branch\n", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
    local second = run_scan(run_opts)
    t.eq(second.exit_code, 0)
    t.eq(h.count_calls("merge --no-ff --no-commit"), 2)
  end,
}
