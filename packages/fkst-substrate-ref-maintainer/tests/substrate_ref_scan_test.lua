local t = fkst.test
local core = require("core")
local gh_argv = require("testkit.gh_argv_mock")
gh_argv.install(t, core)

local current_pin = "cccccccccccccccccccccccccccccccccccccccc"
local target_sha = "1234567890abcdef1234567890abcdef12345678"
local older_valid_pin = "2125600000000000000000000000000000000000"
local base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
local old_branch_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
local pr_head_sha = "dddddddddddddddddddddddddddddddddddddddd"
local pr_number = 27
local substrate_repo = "ChronoAIProject/fkst-substrate"

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

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function ensure_dir(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if ok ~= true and ok ~= 0 then
    error("github-devloop: test directory setup failed")
  end
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
  t.mock_command("git ls-remote https://github.com/ChronoAIProject/fkst-substrate.git refs/heads/dev", {
    stdout = tostring(sha) .. "\trefs/heads/dev\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git ls-remote 'https://github.com/ChronoAIProject/fkst-substrate.git' 'refs/heads/dev'", {
    stdout = tostring(sha) .. "\trefs/heads/dev\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_substrate_check_runs(sha, status, conclusion)
  local conclusion_json = conclusion == nil and "null" or ('"' .. tostring(conclusion) .. '"')
  t.mock_command(core.gh_commit_check_runs_cmd(substrate_repo, sha), {
    stdout = '{"total_count":1,"check_runs":[{"name":"test","status":"'
      .. tostring(status)
      .. '","conclusion":'
      .. conclusion_json
      .. '}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_substrate_check_runs_green(sha, times)
  for _ = 1, times or 1 do
    mock_substrate_check_runs(sha, "completed", "success")
  end
end

local function mock_current_pin(sha)
  t.mock_command("git show HEAD:.fkst/substrate-ref", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_missing_pin()
  t.mock_command("git show HEAD:.fkst/substrate-ref", {
    stdout = "",
    stderr = "fatal: path '.fkst/substrate-ref' does not exist in 'HEAD'\n",
    exit_code = 128,
  })
end

local function mock_pin_read_failure()
  t.mock_command("git show HEAD:.fkst/substrate-ref", {
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
  t.mock_command("git fetch origin dev", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", {
    stdout = base_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_bump_branch_base_ancestry(exit_code)
  t.mock_command("git merge-base --is-ancestor " .. base_sha .. " " .. old_branch_sha, {
    stdout = "",
    stderr = "",
    exit_code = exit_code or 0,
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
  t.mock_command("git fetch origin chore/substrate-ref-bump", {
    stdout = "",
    stderr = "fatal: couldn't find remote ref chore/substrate-ref-bump\n",
    exit_code = 128,
  })
end

local function mock_branch_present()
  t.mock_command("git fetch origin chore/substrate-ref-bump", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/origin/chore/substrate-ref-bump^{commit}", {
    stdout = old_branch_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/chore/substrate-ref-bump^{commit}'", {
    stdout = old_branch_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
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

local function mock_branch_present_at(sha)
  t.mock_command("git fetch origin chore/substrate-ref-bump", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/origin/chore/substrate-ref-bump^{commit}", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/chore/substrate-ref-bump^{commit}'", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git fetch 'origin' 'chore/substrate-ref-bump'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/'origin'/'chore/substrate-ref-bump'^{commit}", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_pin(sha)
  t.mock_command("git show " .. old_branch_sha .. ":.fkst/substrate-ref", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git show '" .. old_branch_sha .. ":.fkst/substrate-ref'", {
    stdout = tostring(sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_pin_for_head(head_sha, pin)
  t.mock_command("git show " .. tostring(head_sha) .. ":.fkst/substrate-ref", {
    stdout = tostring(pin) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git show '" .. tostring(head_sha) .. ":.fkst/substrate-ref'", {
    stdout = tostring(pin) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_pin_missing()
  t.mock_command("git show " .. old_branch_sha .. ":.fkst/substrate-ref", {
    stdout = "",
    stderr = "fatal: path '.fkst/substrate-ref' exists on disk, but not in '" .. old_branch_sha .. "'\n",
    exit_code = 128,
  })
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
  t.mock_command("git worktree remove --force /tmp/fkst-packages-test/github-devloop/stale-substrate", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_worktree_commands(runtime_name, push_with_lease, expected_old_sha)
  local worktree = "/tmp/fkst-packages-test/github-devloop/"
    .. tostring(runtime_name)
    .. "/worktrees/substrate-ref-owner-repo-"
    .. target_sha:sub(1, 12)
  ensure_dir(worktree .. "/.fkst")
  t.mock_command("test -d /tmp/fkst-packages-test/github-devloop/", {
    stdout = "",
    stderr = "",
    exit_code = 1,
  })
  t.mock_command("mkdir -p /tmp/fkst-packages-test/github-devloop/", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git worktree add -B chore/substrate-ref-bump", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p /tmp/fkst-packages-test/github-devloop/", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git -C /tmp/fkst-packages-test/github-devloop/", {
    stdout = ".fkst/substrate-ref\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git -C /tmp/fkst-packages-test/github-devloop/", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git -C /tmp/fkst-packages-test/github-devloop/", {
    stdout = "[chore/substrate-ref-bump 5555555] chore: bump fkst-substrate pin\n",
    stderr = "",
    exit_code = 0,
  })
  if push_with_lease then
    t.mock_command("--force-with-lease=refs/heads/chore/substrate-ref-bump:" .. tostring(expected_old_sha or old_branch_sha), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  else
    t.mock_command("git -C ", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(" push origin HEAD:refs/heads/chore/substrate-ref-bump", {
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
  t.mock_command("gh pr create --repo owner/repo --head chore/substrate-ref-bump --base dev --title 'chore: bump fkst-substrate pin'", {
    stdout = "https://github.com/owner/repo/pull/27\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr create --repo owner/repo --head chore/substrate-ref-bump --base dev --title chore: bump fkst-substrate pin", {
    stdout = "https://github.com/owner/repo/pull/27\n",
    stderr = "",
    exit_code = 0,
  })
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

local function mock_bump_pr_view(comments, extra)
  extra = extra or {}
  local state = extra.state or "OPEN"
  local merged_at = extra.merged_at or ""
  local head_sha = extra.head_sha or pr_head_sha
  local is_draft = extra.is_draft == true and "true" or "false"
  local mergeable = extra.mergeable or "MERGEABLE"
  local merge_state = extra.merge_state or "CLEAN"
  local rollup = extra.rollup or '[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]'
  t.mock_command("gh pr view '27' --repo 'owner/repo' --json headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"chore/substrate-ref-bump","headRefOid":"%s","baseRefName":"dev","baseRefOid":"%s","state":"%s","updatedAt":"2026-06-16T22:10:00Z","isDraft":%s,"mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":%s}\n',
      head_sha,
      base_sha,
      state,
      is_draft,
      merged_at,
      comments and render_comment(comments) or "",
      mergeable,
      merge_state,
      rollup
    ),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr view 27 --repo owner/repo --json 'headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup'", {
    stdout = string.format(
      '{"headRefName":"chore/substrate-ref-bump","headRefOid":"%s","baseRefName":"dev","baseRefOid":"%s","state":"%s","updatedAt":"2026-06-16T22:10:00Z","isDraft":%s,"mergedAt":"%s","comments":[%s],"headRepository":{"nameWithOwner":"owner/repo"},"headRepositoryOwner":{"login":"owner"},"isCrossRepository":false,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":%s}\n',
      head_sha,
      base_sha,
      state,
      is_draft,
      merged_at,
      comments and render_comment(comments) or "",
      mergeable,
      merge_state,
      rollup
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
  t.mock_command("gh pr diff 27 --repo owner/repo --name-only", {
    stdout = (path or ".fkst/substrate-ref") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_branch_head_for_merge(sha, pin)
  t.mock_command("git fetch origin chore/substrate-ref-bump", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git fetch 'origin' 'chore/substrate-ref-bump'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/chore/substrate-ref-bump^{commit}'", {
    stdout = tostring(sha or pr_head_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify refs/remotes/'origin'/'chore/substrate-ref-bump'^{commit}", {
    stdout = tostring(sha or pr_head_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git show " .. tostring(sha or pr_head_sha) .. ":.fkst/substrate-ref", {
    stdout = tostring(pin or target_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git show '" .. tostring(sha or pr_head_sha) .. ":.fkst/substrate-ref'", {
    stdout = tostring(pin or target_sha) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_substrate_pin_ancestor(pin, exit_code)
  t.mock_command("git fetch https://github.com/ChronoAIProject/fkst-substrate.git refs/heads/dev:refs/remotes/fkst-substrate/dev", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git fetch 'https://github.com/ChronoAIProject/fkst-substrate.git' 'refs/heads/dev:refs/remotes/fkst-substrate/dev'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/fkst-substrate/dev^{commit}'", {
    stdout = target_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor " .. tostring(pin or target_sha) .. " " .. target_sha, {
    stdout = "",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function mock_merge_success()
  t.mock_command("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  mock_bump_pr_view(nil, {
    state = "MERGED",
    merged_at = "2026-06-16T22:30:00Z",
  })
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if gh_argv.call_contains(call, needle) then
      count = count + 1
    end
  end
  return count
end

local function count_git_write_calls()
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    local rendered = gh_argv.call_rendered(call)
    if rendered:find("git worktree add", 1, true) ~= nil
      or rendered:find(" git add ", 1, true) ~= nil
      or rendered:find(" commit ", 1, true) ~= nil
      or rendered:find("git push", 1, true) ~= nil
      or rendered:find(" push origin ", 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function count_raises(result, queue)
  local count = 0
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function eq_zero(value, label)
  if value ~= 0 then
    error(tostring(label) .. ": expected 0, got " .. tostring(value))
  end
end

return {
  test_missing_substrate_ref_pin_is_benign_noop = function()
    mock_env("")
    mock_missing_pin()

    local result = run_scan(opts("substrate-no-pin"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show HEAD:.fkst/substrate-ref"), 1)
    t.eq(count_calls("git ls-remote"), 0)
    t.eq(count_calls("gh api"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_git_write_calls(), 0)
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
    t.eq(count_git_write_calls(), 0)
  end,

  test_dry_run_plans_singleton_bump_without_writes = function()
    mock_env("")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha)
    mock_no_existing_pr()

    local result = run_scan(opts("substrate-dry-run"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump")), 1)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_git_write_calls(), 0)
  end,

  test_dry_run_holds_unpublishable_substrate_head_without_writes = function()
    mock_env("")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_no_existing_pr()
    mock_substrate_check_runs(target_sha, "in_progress", nil)

    local result = run_scan(opts("substrate-unpublishable-dry-run"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_commit_check_runs_cmd(substrate_repo, target_sha)), 1)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_git_write_calls(), 0)
  end,

  test_real_mode_holds_unpublishable_substrate_head_before_branch_mutation = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_no_existing_pr()
    mock_substrate_check_runs(target_sha, "completed", "failure")

    local result = run_scan(opts("substrate-unpublishable-real", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_commit_check_runs_cmd(substrate_repo, target_sha)), 1)
    eq_zero(count_calls("git worktree add"), "worktree add for unpublishable target")
    eq_zero(count_calls("gh pr create"), "PR create for unpublishable target")
    eq_zero(count_calls("HEAD:refs/heads/chore/substrate-ref-bump"), "push for unpublishable target")
  end,

  test_real_mode_creates_single_bump_pr_for_new_dev_head = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 3)
    mock_no_existing_pr()
    mock_branch_missing()
    mock_base_head()
    mock_runtime_root("substrate-create")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands("substrate-create", false)
    mock_pr_create()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_merge_success()

    local result = run_scan(opts("substrate-create", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("HEAD:refs/heads/chore/substrate-ref-bump"), 1)
    t.eq(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), 1)
    local audit_raise = result.raises[1]
    t.eq(audit_raise.queue, "github-proxy.github_pr_comment_request")
    t.eq(audit_raise.payload.pr_number, pr_number)
    t.is_true(audit_raise.payload.body:find("github-devloop substrate-ref deterministic merge audit", 1, true) ~= nil)
    t.is_true(audit_raise.payload.body:find("fkst:github-devloop:substrate-ref-merge:v1", 1, true) ~= nil)
    t.is_true(audit_raise.payload.body:find('target_sha="' .. target_sha .. '"', 1, true) ~= nil)
    eq_zero(count_raises(result, "github-proxy.github_issue_create_request"), "create raises after new bump")
    eq_zero(count_raises(result, "github-proxy.github_issue_label_request"), "label raises after new bump")
  end,

  test_real_mode_merges_existing_green_bump_pr_with_own_valid_pin_before_repinning = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(older_valid_pin, 2)
    mock_existing_pr()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, older_valid_pin)
    mock_substrate_pin_ancestor(older_valid_pin)
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, older_valid_pin)
    mock_substrate_pin_ancestor(older_valid_pin)
    mock_merge_success()

    local result = run_scan(opts("substrate-update", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    eq_zero(count_calls("git worktree add"), "worktree add for valid existing bump")
    eq_zero(count_calls("git worktree remove --force"), "worktree remove for valid existing bump")
    eq_zero(count_calls("gh pr create"), "PR create for valid existing bump")
    eq_zero(count_calls("--force-with-lease=refs/heads/chore/substrate-ref-bump:" .. old_branch_sha), "push lease for valid existing bump")
    t.eq(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    t.is_true(result.raises[1].payload.body:find("substrate-ref-merge:v1", 1, true) ~= nil)
    eq_zero(count_raises(result, "github-proxy.github_issue_create_request"), "create raises after valid existing bump")
    eq_zero(count_raises(result, "github-proxy.github_issue_label_request"), "label raises after valid existing bump")
  end,

  test_real_mode_rechecks_pr_under_lock_before_update = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 2)
    mock_existing_pr()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_merge_success()

    local result = run_scan(opts("substrate-recheck", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls(core.gh_pr_list_head_cmd("owner/repo", "chore/substrate-ref-bump")), 1)
    eq_zero(count_calls("gh pr create"), "PR create during recheck")
    eq_zero(count_calls(" push origin HEAD:refs/heads/'chore/substrate-ref-bump'"), "quoted push during recheck")
    eq_zero(count_calls("git worktree add"), "worktree add during recheck")
    eq_zero(count_calls("git worktree remove --force"), "worktree remove during recheck")
    t.eq(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
    eq_zero(count_raises(result, "github-proxy.github_issue_create_request"), "create raises during recheck")
    eq_zero(count_raises(result, "github-proxy.github_issue_label_request"), "label raises during recheck")
  end,

  test_real_mode_merges_existing_green_bump_pr_before_checking_already_current_branch = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 2)
    mock_existing_pr()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_merge_success()

    local result = run_scan(opts("substrate-already-current", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    eq_zero(count_calls("gh pr create"), "PR create before already-current merge")
    eq_zero(count_calls("git worktree add"), "worktree add before already-current merge")
    eq_zero(count_calls("git worktree remove --force"), "worktree remove before already-current merge")
    eq_zero(count_calls("git push"), "git push before already-current merge")
    t.eq(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), 1)
    t.eq(result.raises[1].queue, "github-proxy.github_pr_comment_request")
  end,

  test_real_mode_holds_existing_bump_pr_when_diff_is_not_exact_pin_file = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin(target_sha)
    mock_base_head()
    mock_bump_branch_base_ancestry(0)
    mock_bump_pr_view()
    mock_bump_diff(".fkst/substrate-ref\nREADME.md")

    local result = run_scan(opts("substrate-unexpected-diff", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    eq_zero(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), "merge call for unexpected diff")
    eq_zero(count_raises(result, "github-proxy.github_pr_comment_request"), "comment raises for unexpected diff")
  end,

  test_real_mode_repins_existing_bump_pr_when_pin_is_not_substrate_dev_ancestor = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 2)
    mock_existing_pr()
    mock_branch_present()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, current_pin)
    mock_substrate_pin_ancestor(current_pin, 1)
    mock_branch_present()
    mock_branch_pin_missing()
    mock_base_head()
    mock_runtime_root("substrate-pin-mismatch")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands("substrate-pin-mismatch", true, old_branch_sha)
    mock_existing_pr()
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"IN_PROGRESS","conclusion":""}]',
    })
    mock_bump_diff()
    mock_branch_present_at(pr_head_sha)
    mock_branch_pin_for_head(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)

    local result = run_scan(opts("substrate-pin-mismatch", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("--force-with-lease=refs/heads/chore/substrate-ref-bump:" .. old_branch_sha), 1)
    eq_zero(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), "merge call after repin")
    eq_zero(count_raises(result, "github-proxy.github_pr_comment_request"), "comment raises after repin")
  end,

  test_real_mode_holds_existing_bump_pr_when_ci_is_not_green = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 2)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin(target_sha)
    mock_base_head()
    mock_bump_branch_base_ancestry(0)
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"COMPLETED","conclusion":"FAILURE"}]',
    })
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"COMPLETED","conclusion":"FAILURE"}]',
    })
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)

    local result = run_scan(opts("substrate-ci-red", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    eq_zero(count_calls("gh pr merge"), "merge call for red CI")
    eq_zero(count_raises(result, "github-proxy.github_pr_comment_request"), "comment raises for red CI")
  end,

  test_real_mode_refreshes_existing_bump_branch_when_pin_matches_but_base_is_stale = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha)
    mock_existing_pr()
    mock_branch_present()
    mock_branch_pin(target_sha)
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"COMPLETED","conclusion":"FAILURE"}]',
    })
    mock_bump_diff(".fkst/substrate-ref\nREADME.md")
    mock_base_head()
    mock_bump_branch_base_ancestry(1)
    mock_runtime_root("substrate-stale-base")
    mock_no_checked_out_bump_branch()
    mock_worktree_commands("substrate-stale-base", true, old_branch_sha)
    mock_existing_pr()
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"IN_PROGRESS","conclusion":""}]',
    })
    mock_bump_diff(".fkst/substrate-ref\nREADME.md")

    local result = run_scan(opts("substrate-stale-base", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git merge-base --is-ancestor " .. base_sha .. " " .. old_branch_sha), 1)
    t.eq(count_calls("git worktree add -B chore/substrate-ref-bump"), 1)
    t.eq(count_calls("--force-with-lease=refs/heads/chore/substrate-ref-bump:" .. old_branch_sha), 1)
    eq_zero(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), "merge call after stale-base refresh")
    eq_zero(count_raises(result, "github-proxy.github_pr_comment_request"), "comment raises after stale-base refresh")
  end,

  test_real_mode_removes_stale_checked_out_bump_branch_worktree_before_update = function()
    mock_env("1")
    mock_current_pin(current_pin)
    mock_substrate_head(target_sha)
    mock_substrate_check_runs_green(target_sha, 2)
    mock_existing_pr()
    mock_branch_present()
    mock_bump_pr_view()
    mock_bump_diff()
    mock_branch_head_for_merge(pr_head_sha, current_pin)
    mock_substrate_pin_ancestor(current_pin, 1)
    mock_branch_present()
    mock_branch_pin_missing()
    mock_base_head()
    mock_runtime_root("substrate-stale-worktree")
    mock_checked_out_bump_branch()
    mock_worktree_commands("substrate-stale-worktree", true, old_branch_sha)
    mock_existing_pr()
    mock_bump_pr_view(nil, {
      rollup = '[{"name":"ci","status":"IN_PROGRESS","conclusion":""}]',
    })
    mock_bump_diff()
    mock_branch_present_at(pr_head_sha)
    mock_branch_pin_for_head(pr_head_sha, target_sha)
    mock_substrate_pin_ancestor(target_sha)

    local result = run_scan(opts("substrate-stale-worktree", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git worktree remove --force /tmp/fkst-packages-test/github-devloop/stale-substrate"), 1)
    t.eq(count_calls("--force-with-lease=refs/heads/chore/substrate-ref-bump:" .. old_branch_sha), 1)
    eq_zero(count_calls("gh pr merge '27' --repo 'owner/repo' --merge --match-head-commit '" .. pr_head_sha .. "'"), "merge call after stale-worktree repin")
    eq_zero(count_raises(result, "github-proxy.github_issue_create_request"), "create raises after stale-worktree repin")
    eq_zero(count_raises(result, "github-proxy.github_issue_label_request"), "label raises after stale-worktree repin")
  end,
}
