local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function mock_bot(login, write_mode)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = login or "fkst-test-bot",
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

local function assignees_json(logins)
  local rendered = {}
  for _, login in ipairs(logins or {}) do
    table.insert(rendered, string.format('{"login":"%s"}', tostring(login)))
  end
  return '{"assignees":[' .. table.concat(rendered, ",") .. "]}\n"
end

local function state(name, created_at)
  return {
    state = name or "thinking",
    version = "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    marker_created_at = created_at or "1970-01-01T00:00:00Z",
  }
end

return {
  test_issue_claim_state_is_current_assignees_only = function()
    t.eq(core.issue_claim_state({}, "fkst-test-bot"), "unassigned")
    t.eq(core.issue_claim_state({ { login = "fkst-test-bot" } }, "fkst-test-bot"), "self")
    t.eq(core.issue_claim_state({ { login = "human" } }, "fkst-test-bot"), "other")
    t.eq(core.issue_claim_state({ { login = "fkst-test-bot" }, { login = "other-bot" } }, "fkst-test-bot"), "other")
  end,

  test_dry_run_claim_proceeds_without_assigning = function()
    mock_bot("fkst-test-bot", "")

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = {} },
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, true)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_claim_assigns_then_verifies_self_only_winner = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue view '42' --repo 'owner/repo' --json assignees", {
      stdout = assignees_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = {} },
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, true)
    t.eq(count_calls("--add-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 0)
  end,

  test_claim_loss_unassigns_only_self_and_skips = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue view '42' --repo 'owner/repo' --json assignees", {
      stdout = assignees_json({ "other-bot" }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = {} },
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, false)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
    t.eq(count_calls("--remove-assignee 'other-bot'"), 0)
  end,

  test_non_self_assignee_is_never_touched = function()
    mock_bot("fkst-test-bot", "1")

    local ok = core.claim_issue_for_management(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "human" } } },
      "github-devloop/issue/owner/repo/42"
    )

    t.eq(ok, false)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_timeout_release_requires_fresh_self_only_read = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue view '42' --repo 'owner/repo' --json assignees", {
      stdout = assignees_json({ "fkst-test-bot" }),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit '42' --repo 'owner/repo' --remove-assignee 'fkst-test-bot'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local released = core.maybe_release_stale_self_claim(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "fkst-test-bot" } } },
      "github-devloop/issue/owner/repo/42",
      state("thinking")
    )

    t.eq(released, true)
    t.eq(count_calls("gh issue view '42' --repo 'owner/repo' --json assignees"), 1)
    t.eq(count_calls("--remove-assignee 'fkst-test-bot'"), 1)
  end,

  test_timeout_release_skips_when_fresh_read_is_not_self_only = function()
    mock_bot("fkst-test-bot", "1")
    t.mock_command("gh issue view '42' --repo 'owner/repo' --json assignees", {
      stdout = assignees_json({ "fkst-test-bot", "human" }),
      stderr = "",
      exit_code = 0,
    })

    local released = core.maybe_release_stale_self_claim(
      "claim_contract",
      "owner/repo",
      42,
      { assignees = { { login = "fkst-test-bot" } } },
      "github-devloop/issue/owner/repo/42",
      state("thinking")
    )

    t.eq(released, false)
    t.eq(count_calls("--remove-assignee"), 0)
  end,
}
