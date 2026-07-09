local M = {}

local gh_argv = require("testkit.gh_argv_mock")

local function configure_test_bot_login()
  local ok, devloop_base = pcall(require, "devloop.base")
  if ok and type(devloop_base) == "table" and type(devloop_base.configure_trusted_bot_login) == "function" then
    devloop_base.configure_trusted_bot_login("fkst-test-bot")
  end
end

local function mock_author_policy_env(t)
  configure_test_bot_login()
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', {
    stdout = "fkst-test-bot,ElonSG",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_AUTHORIZED_LOGINS"', {
    stdout = "trusted-human",
    stderr = "",
    exit_code = 0,
  })
end

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

function M.new(deps)
  deps = deps or {}
  local core = deps.core or error("testkit.devloop_core_fixtures: deps.core is required")
  local t = deps.t or fkst.test

  gh_argv.install(t, core)
  configure_test_bot_login()
  mock_author_policy_env(t)

  local function source_ref()
    return {
      kind = "external",
      ref = "owner/repo#issue/42",
    }
  end

  local function issue(extra)
    local fields = extra or {}
    local updated_at = fields.updated_at or "2026-06-03T01:02:03Z"
    local value = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = 42,
      title = "Implement decision recorder",
      url = "https://github.example/owner/repo/issues/42",
      state = "OPEN",
      updated_at = updated_at,
      labels = { "fkst-dev:enabled" },
      dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
      source_ref = source_ref(),
    }
    for key, field in pairs(fields) do
      value[key] = field
    end
    return value
  end

  local function reached(extra)
    local value = {
      schema = "consensus.consensus_reached.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      decision = "approve",
      body = "All angles approve.",
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      source_ref = source_ref(),
    }
    for key, field in pairs(extra or {}) do
      value[key] = field
    end
    return value
  end

  local function unresolved(extra)
    local value = {
      schema = "consensus.consensus_converge.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      source_ref = source_ref(),
    }
    for key, field in pairs(extra or {}) do
      value[key] = field
    end
    return value
  end

  return {
    core = core,
    t = t,
    has_value = has_value,
    source_ref = source_ref,
    issue = issue,
    reached = reached,
    unresolved = unresolved,
    argv_rendered = gh_argv.argv_rendered,
    mock_author_policy_env = function()
      mock_author_policy_env(t)
    end,
  }
end

return M
