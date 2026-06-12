local S = {}

function S.install(M)
local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
  FKST_DEVLOOP_UPSTREAM_BRANCH = true,
  FKST_DEVLOOP_INTEGRATION_BRANCH = true,
  FKST_DEVLOOP_MAX_INFLIGHT = true,
  FKST_DEVLOOP_ROLLUP_MERGE = true,
  FKST_DEVLOOP_TEST_COMMAND = true,
  FKST_OUTPUT_LANG = true,
}

local allowed_presence_env = {
  GH_TOKEN = true,
  GITHUB_TOKEN = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-devloop: env name is not allowed")
  end
  return 'printf %s "$' .. name .. '"'
end

local function env_present_command(name)
  if not allowed_presence_env[name] then
    error("github-devloop: env name is not allowed")
  end
  return 'if [ -n "${' .. name .. ':-}" ]; then printf present; fi'
end

function M.read_env_command(name)
  return read_env_command(name)
end

function M.env_present_command(name)
  return env_present_command(name)
end

function M.read_env(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    return nil
  end
  local ok, out = pcall(run, read_env_command(name))
  if not ok or type(out) ~= "table" or out.exit_code ~= 0 or out.stdout == "" then
    return nil
  end
  return out.stdout
end

function M.env_present(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    return false
  end
  local ok, out = pcall(run, env_present_command(name))
  return ok and type(out) == "table" and out.exit_code == 0 and out.stdout ~= ""
end

function M.write_mode(exec)
  return M.read_env("FKST_GITHUB_WRITE", exec) == "1" and "real" or "dry-run"
end

function M.max_inflight(exec)
  local value = M.read_env("FKST_DEVLOOP_MAX_INFLIGHT", exec)
  if value == nil then
    return nil
  end
  value = M._trim(value)
  if value == "" then
    return nil
  end
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) or parsed < 1 or parsed > 100 then
    error("github-devloop: invalid FKST_DEVLOOP_MAX_INFLIGHT")
  end
  return parsed
end

function M.max_fix_rounds()
  return 12
end

function M.max_converge_rounds()
  return 8
end

function M.default_test_command()
  return "scripts/run.sh test"
end

function M.test_command(exec)
  local command = M.read_env("FKST_DEVLOOP_TEST_COMMAND", exec)
  if command == nil then
    return M.default_test_command()
  end
  return command
end

local function current_checkout_branch(exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("github-devloop: branch config requires exec_sync")
  end
  local ok, out = pcall(run, "git rev-parse --abbrev-ref HEAD")
  if not ok or type(out) ~= "table" or out.exit_code ~= 0 then
    error("github-devloop: current checkout branch read failed")
  end
  local branch = M._trim(out.stdout)
  if branch == "HEAD" or not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid current checkout branch")
  end
  return branch
end

local function validated_branch(name, branch)
  branch = M._trim(branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid " .. name)
  end
  return branch
end

function M.branch_config(exec)
  local upstream_env = M.read_env("FKST_DEVLOOP_UPSTREAM_BRANCH", exec)
  local upstream = upstream_env
  if upstream == nil then
    upstream = current_checkout_branch(exec)
  end
  upstream = validated_branch("FKST_DEVLOOP_UPSTREAM_BRANCH", upstream)
  local integration = M.read_env("FKST_DEVLOOP_INTEGRATION_BRANCH", exec)
  if integration == nil then
    integration = upstream
  end
  integration = validated_branch("FKST_DEVLOOP_INTEGRATION_BRANCH", integration)
  return {
    upstream = upstream,
    integration = integration,
  }
end

function M.devloop_config(exec)
  local branches = M.branch_config(exec)
  local rollup_merge = M.read_env("FKST_DEVLOOP_ROLLUP_MERGE", exec) or "auto"
  rollup_merge = M._trim(rollup_merge)
  if rollup_merge ~= "auto" and rollup_merge ~= "manual" then
    error("github-devloop: invalid FKST_DEVLOOP_ROLLUP_MERGE")
  end
  return {
    repo = M.read_env("FKST_GITHUB_REPO", exec),
    bot_login = M.read_env("FKST_GITHUB_BOT_LOGIN", exec),
    write_mode = M.write_mode(exec),
    upstream_branch = branches.upstream,
    integration_branch = branches.integration,
    rollup_merge = rollup_merge,
  }
end
end

return S
