local t = fkst.test
local core = require("core")

local current_pin = "cccccccccccccccccccccccccccccccccccccccc"
local target_sha = "1234567890abcdef1234567890abcdef12345678"
local base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
local old_branch_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
local pr_head_sha = "dddddddddddddddddddddddddddddddddddddddd"
local backing_issue_number = 845

local function opts(name, extra)
  local env = {
    FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    FKST_GITHUB_WRITE = "",
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
  }
  for key, value in pairs(extra or {}) do
    env[key] = value
  end
  return { env = env }
end

local function run_scan(run_opts)
  return t.run_department("departments/substrate_ref_scan/main.lua", {
    queue = "devloop_substrate_ref_tick",
    payload = { schema = "github-devloop.substrate-ref-tick.v1" },
  }, run_opts or opts("substrate-ref"))
end

local function mock_env(write_mode)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "integration/dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 3 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = write_mode or "",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 3 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_substrate_head(sha)
  t.mock_command("git ls-remote 'https://github.com/ChronoAIProject/fkst-substrate.git' 'refs/heads/dev'", {
    stdout = tostring(sha) .. "\trefs/heads/dev\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_current_pin(sha)
  t.mock_command(core.git_show_substrate_ref_pin_cmd(), {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_missing_pin()
  t.mock_command(core.git_show_substrate_ref_pin_cmd(), {
    stdout = "",
    stderr = "fatal: path '.fkst/substrate-ref' does not exist in 'HEAD'\n",
    exit_code = 128,
  })
end

local function mock_pin_read_failure()
  t.mock_command(core.git_show_substrate_ref_pin_cmd(), {
    stdout = "",
    stderr = "fatal: bad object HEAD\n",
    exit_code = 128,
  })
end

local function mock_no_existing_pr()
  t.mock_command(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump"), {
    stdout = "[[]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_existing_pr()
  t.mock_command(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump"), {
    stdout = '[[{"number":27,"head":{"ref":"chore/substrate-ref-bump"},"base":{"ref":"dev"}}]]\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_base_head()
  t.mock_command("git fetch 'origin' 'dev'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/'origin'/'dev'^{commit}", {
    stdout = base_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_runtime_root(name)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/" .. tostring(name),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_missing()
  t.mock_command("git fetch 'origin' 'chore/substrate-ref-bump'", {
    stdout = "",
    stderr = "fatal: couldn't find remote ref chore/substrate-ref-bump\n",
    exit_code = 128,
  })
end

local function mock_branch_present()
  t.mock_command("git fetch 'origin' 'chore/substrate-ref-bump'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/'origin'/'chore/substrate-ref-bump'^{commit}", {
    stdout = old_branch_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_pin(sha)
  t.mock_command("git show '" .. old_branch_sha .. ":.fkst/substrate-ref'", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_pin_missing()
  t.mock_command("git show '" .. old_branch_sha .. ":.fkst/substrate-ref'", {
    stdout = "",
    stderr = "fatal: path '.fkst/substrate-ref' exists on disk, but not in '" .. old_branch_sha .. "'\n",
    exit_code = 128,
  })
end

local function mock_no_checked_out_bump_branch()
  t.mock_command("git worktree list --porcelain", {
    stdout = "worktree /repo\nHEAD " .. base_sha .. "\nbranch refs/heads/dev\n\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_checked_out_bump_branch()
  t.mock_command("git worktree list --porcelain", {
    stdout = table.concat({
      "worktree /repo",
      "HEAD " .. base_sha,
      "branch refs/heads/dev",
      "",
      "worktree /tmp/fkst-packages-test/github-devloop/stale-substrate",
      "HEAD " .. old_branch_sha,
      "branch refs/heads/chore/substrate-ref-bump",
      "",
    }, "\n"),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git worktree remove --force '/tmp/fkst-packages-test/github-devloop/stale-substrate'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_worktree_commands(push_with_lease)
  t.mock_command("if [ -d '/tmp/fkst-packages-test/github-devloop/", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git worktree add -B 'chore/substrate-ref-bump'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("printf %s '" .. target_sha .. "\n' > ", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git -C ", {
    stdout = ".fkst/substrate-ref\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(" add -A", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(" commit -m 'chore: bump fkst-substrate pin'", {
    stdout = "[chore/substrate-ref-bump 5555555] chore: bump fkst-substrate pin\n",
    stderr = "",
    exit_code = 0,
  })
  if push_with_lease then
    t.mock_command("--force-with-lease='refs/heads/chore/substrate-ref-bump:" .. old_branch_sha .. "'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  else
    t.mock_command(" push origin HEAD:refs/heads/'chore/substrate-ref-bump'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("git worktree remove --force", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_create()
  t.mock_command("gh pr create --repo 'owner/repo' --head 'chore/substrate-ref-bump' --base 'dev' --title 'chore: bump fkst-substrate pin'", {
    stdout = "https://github.com/owner/repo/pull/27\n",
    stderr = "",
    exit_code = 0,
  })
end

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function render_comment(body)
  return string.format(
    '{"body":"%s","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-16T22:10:00Z"}',
    json_string(body)
  )
end

local function substrate_review_proposal()
  return core.pr_review_proposal_id("owner/repo", 27, "substrate-ref-bump/" .. pr_head_sha, pr_head_sha)
end

local function substrate_review_dedup()
  return core._dedup_key({
    "substrate-ref-bump",
    "review",
    core.safe_repo("owner/repo"),
    "27",
    pr_head_sha,
  })
end

local substrate_backing_issue_comment

local function substrate_lifecycle_comment()
  local proposal_id = core.proposal_id("owner/repo", backing_issue_number)
  local version = "substrate-ref-bump/" .. pr_head_sha
  local review_proposal = substrate_review_proposal()
  local review_dedup = substrate_review_dedup()
  return table.concat({
    substrate_backing_issue_comment(),
    core.pr_origin_marker(proposal_id, backing_issue_number, "chore/substrate-ref-bump", version, "dev"),
    core.state_marker(proposal_id, "merge-ready", version),
    core.review_result_marker(review_proposal, proposal_id, "approve", review_dedup),
    core.merge_ready_marker(proposal_id, 27, version, review_proposal, review_dedup, pr_head_sha),
  }, "\n")
end

substrate_backing_issue_comment = function()
  return '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. core._dedup_key({
    "substrate-ref-bump",
    "backing-issue",
    core.safe_repo("owner/repo"),
    "chore/substrate-ref-bump",
  }) .. '" issue="' .. backing_issue_number .. '" -->'
end

local function legacy_substrate_backing_issue_comment()
  return '<!-- fkst:github-proxy:issue-created:v1 dedup="' .. core._dedup_key({
    "substrate-ref-bump",
    "backing-issue",
    core.safe_repo("owner/repo"),
    "27",
  }) .. '" issue="' .. backing_issue_number .. '" -->'
end

local function mock_bump_pr_view(comments)
  t.mock_command("gh pr view '27' --repo 'owner/repo' --json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"chore/substrate-ref-bump","headRefOid":"%s","baseRefName":"dev","baseRefOid":"%s","state":"OPEN","updatedAt":"2026-06-16T22:10:00Z","isDraft":false,"mergedAt":"","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]}\n',
      pr_head_sha,
      base_sha,
      comments and render_comment(comments) or ""
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_bump_diff(path)
  t.mock_command("gh pr diff '27' --repo 'owner/repo' --name-only", {
    stdout = (path or ".fkst/substrate-ref") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

return {
  test_missing_substrate_ref_pin_is_benign_noop = function()
    mock_env("")
    mock_missing_pin()

    local result = run_scan(opts("substrate-no-pin"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.git_show_substrate_ref_pin_cmd()), 1)
    t.eq(count_calls("git ls-remote"), 0)
    t.eq(count_calls("gh api"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("git worktree"), 0)
    t.eq(count_calls("git push"), 0)
  end,

  test_pin_read_git_failure_still_fails_closed = function()
    mock_env("")
    mock_pin_read_failure()

    local result = run_scan(opts("substrate-pin-read-failure"))

    t.eq(result.exit_code, 1)
    t.eq(count_calls("git ls-remote"), 0)
    t.eq(count_calls("gh api"), 0)
  end,

  test_current_pin_performs_no_github_or_git_writes = function()
    mock_env("")
    mock_current_pin(current_pin)
    mock_substrate_head(current_pin)

    local result = run_scan(opts("substrate-current"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("git worktree"), 0)
    t.eq(count_calls("git push"), 0)
  end,

  test_dry_run_plans_singleton_bump_without_writes = function()
    mock_env("")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_no_existing_pr()

    local result = run_scan(opts("substrate-dry-run"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump")), 1)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("git worktree"), 0)
    t.eq(count_calls("git push"), 0)
  end,

  test_real_mode_creates_single_bump_pr_for_new_dev_head = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_no_existing_pr()
    mock_branch_missing()
    mock_base_head()
    mock_runtime_root("substrate-create")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands(false)
    mock_pr_create()
    mock_bump_pr_view()
    mock_bump_diff()

    local result = run_scan(opts("substrate-create", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls(" push origin HEAD:refs/heads/'chore/substrate-ref-bump'"), 1)
    local create_raise = result.raises[1]
    t.eq(create_raise.queue, "github-proxy.github_issue_create_request")
    t.eq(create_raise.payload.parent_comment_target.pr_number, 27)
    t.eq(create_raise.payload.dedup_key, core._dedup_key({
      "substrate-ref-bump",
      "backing-issue",
      core.safe_repo("owner/repo"),
      "chore/substrate-ref-bump",
    }))
    t.is_true(create_raise.payload.body:find("PR: #27", 1, true) ~= nil)
    t.is_true(create_raise.payload.body:find("Authoritative owner:", 1, true) ~= nil)
    t.is_true(create_raise.payload.body:find("Ledger de-duplication:", 1, true) ~= nil)
    t.is_true(create_raise.payload.body:find("Recurrence waiver:", 1, true) ~= nil)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_real_mode_updates_existing_bump_pr_branch_without_creating_second_pr = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin_missing()
    mock_base_head()
    mock_runtime_root("substrate-update")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands(true)
    mock_bump_pr_view(substrate_backing_issue_comment())
    mock_bump_diff()

    local result = run_scan(opts("substrate-update", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("--force-with-lease='refs/heads/chore/substrate-ref-bump:" .. old_branch_sha .. "'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.is_true(result.raises[1].payload.body:find('proposal="' .. core.proposal_id("owner/repo", backing_issue_number) .. '"', 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('issue="' .. tostring(backing_issue_number) .. '"', 1, true) ~= nil)
    t.is_true(result.raises[1].payload.body:find('state="merge-ready"', 1, true) ~= nil)
  end,

  test_real_mode_honors_legacy_pr_number_backing_issue_ledger = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin_missing()
    mock_base_head()
    mock_runtime_root("substrate-legacy-ledger")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands(true)
    mock_bump_pr_view(legacy_substrate_backing_issue_comment())
    mock_bump_diff()

    local result = run_scan(opts("substrate-legacy-ledger", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.is_true(result.raises[1].payload.body:find('proposal="' .. core.proposal_id("owner/repo", backing_issue_number) .. '"', 1, true) ~= nil)
  end,

  test_real_mode_rechecks_pr_under_lock_before_create = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_branch_missing()
    mock_base_head()
    mock_runtime_root("substrate-recheck")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands(false)
    mock_bump_pr_view(substrate_backing_issue_comment())
    mock_bump_diff()

    local result = run_scan(opts("substrate-recheck", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump")), 1)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls(" push origin HEAD:refs/heads/'chore/substrate-ref-bump'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
  end,

  test_real_mode_skips_push_when_bump_branch_already_targets_dev_head = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin(target_sha)
    mock_bump_pr_view(substrate_lifecycle_comment())
    mock_bump_diff()

    local result = run_scan(opts("substrate-already-current", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("git worktree"), 0)
    t.eq(count_calls("git push"), 0)
    t.eq(result.raises[1].queue, "devloop_merge_ready")
    t.eq(result.raises[1].payload.proposal_id, core.proposal_id("owner/repo", backing_issue_number))
    t.eq(result.raises[1].payload.review_proposal_id, substrate_review_proposal())
  end,

  test_real_mode_removes_stale_checked_out_bump_branch_worktree_before_update = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin_missing()
    mock_base_head()
    mock_runtime_root("substrate-stale-worktree")
    mock_checked_out_bump_branch()
    mock_worktree_commands(true)
    mock_bump_pr_view(substrate_backing_issue_comment())
    mock_bump_diff()

    local result = run_scan(opts("substrate-stale-worktree", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git worktree remove --force '/tmp/fkst-packages-test/github-devloop/stale-substrate'"), 1)
    t.eq(count_calls("--force-with-lease='refs/heads/chore/substrate-ref-bump:" .. old_branch_sha .. "'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
  end,
}
