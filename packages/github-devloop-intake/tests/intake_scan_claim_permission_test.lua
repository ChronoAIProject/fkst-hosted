local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local gh_argv = require("testkit.gh_argv_mock")

local function mock_bot_env()
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 6 do
    t.mock_command('printf %s "$FKST_GITHUB_CLAIM_MODE"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 6 do
    t.mock_command('printf %s "$FKST_DEVLOOP_FORK_GRACE_HOURS"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_repo_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "dev",
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
end

local function mock_write_mode_reads(count)
  for _ = 1, count do
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
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

local function mock_scan_view(number, labels)
  entity_read_mocks.mock_issue_view_selector(t, {
    number = number,
    title = "Issue " .. tostring(number),
    state = "OPEN",
    labels = labels or {},
    comments = {},
    assignees = {},
    author_login = "fkst-test-bot",
  }, "title,labels,comments,state,assignees,author")
end

local function run_scan()
  return t.run_department("departments/intake_scan/main.lua", {
    queue = "devloop_intake_tick",
    payload = { schema = "github-devloop.intake-tick.v1" },
  }, opts("intake-scan-claim-permission-denied"))
end

return {
  test_permission_denied_claim_skips_issue_and_continues_poll = function()
    mock_bot_env()
    mock_repo_env()
    mock_write_mode_reads(6)
    entity_read_mocks.mock_issue_list_command(t, core.gh_issue_list_intake_cmd("owner/repo", 100), {
      { number = 42, labels = {}, assignees = {}, author_login = "fkst-test-bot" },
      { number = 43, labels = {}, assignees = {}, author_login = "fkst-test-bot" },
    })
    mock_scan_view(42, {})
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "GraphQL: Resource not accessible by integration (permission-denied)\n",
      exit_code = 1,
    })
    mock_scan_view(43, {})
    t.mock_command("gh issue edit '43' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", "43"), {
      stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_scan()

    t.eq(result.exit_code, 0)
    local candidates = {}
    for _, raised in ipairs(result.raises or {}) do
      if raised.queue == "devloop_intake_candidate" then
        candidates[tostring(raised.payload.issue_number)] = raised
      end
    end
    t.eq(candidates["42"], nil)
    t.is_true(candidates["43"] ~= nil)
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 2)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 0)
  end,
}
