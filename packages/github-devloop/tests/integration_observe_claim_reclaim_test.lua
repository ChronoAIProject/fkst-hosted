local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local run_observe = h.run_observe
local mock_issue_state = h.mock_issue_state
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise

local function mock_claim_env()
  mock_bot_env()
  for _ = 1, 6 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 6 do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function assignees_json(logins)
  local rendered = {}
  for _, login in ipairs(logins or {}) do
    table.insert(rendered, string.format('{"login":"%s"}', h.json_string(login)))
  end
  return '{"assignees":[' .. table.concat(rendered, ",") .. "]}\n"
end

local function mock_add_self()
  t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_remove_self()
  t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_claim_view(logins)
  t.mock_command("gh issue view '42' --repo 'owner/repo' --json assignees", {
    stdout = assignees_json(logins),
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_managed_unassigned_issue_is_reclaimed_before_replay = function()
    mock_claim_env()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", nil, {})
    mock_add_self()
    mock_claim_view({ "fkst-test-bot" })

    local result = run_observe(issue(), opts("observe-managed-unassigned-reclaim", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal").payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/replay")
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 0)
    t.eq(count_calls("gh issue view '42' --repo 'owner/repo' --json assignees"), 1)
  end,

  test_timeout_release_reclaims_issue_before_replay = function()
    mock_claim_env()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", nil, { "fkst-test-bot" })
    mock_claim_view({ "fkst-test-bot" })
    mock_remove_self()
    mock_add_self()
    mock_claim_view({ "fkst-test-bot" })

    local result = run_observe(issue(), opts("observe-timeout-release-reclaim", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal").payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/replay")
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("gh issue view '42' --repo 'owner/repo' --json assignees"), 2)
  end,
}
