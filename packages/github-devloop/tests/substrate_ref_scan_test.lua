local t = fkst.test
local core = require("core")

local current_pin = "bce8afb5ce11577b8b3f9a3ea4cceffcd362077a"
local target_sha = "1234567890abcdef1234567890abcdef12345678"
local base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
local old_branch_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

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
  test_current_pin_performs_no_github_or_git_writes = function()
    mock_env("")
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
    mock_substrate_head(target_sha)
    mock_no_existing_pr()
    mock_base_head()
    mock_runtime_root("substrate-create")
    mock_branch_missing()
    mock_worktree_commands(false)
    mock_pr_create()

    local result = run_scan(opts("substrate-create", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls(" push origin HEAD:refs/heads/'chore/substrate-ref-bump'"), 1)
  end,

  test_real_mode_updates_existing_bump_pr_branch_without_creating_second_pr = function()
    mock_env("1")
    mock_substrate_head(target_sha)
    mock_existing_pr()
    mock_base_head()
    mock_runtime_root("substrate-update")
    mock_branch_present()
    mock_worktree_commands(true)

    local result = run_scan(opts("substrate-update", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("--force-with-lease='refs/heads/chore/substrate-ref-bump:" .. old_branch_sha .. "'"), 1)
  end,
}
