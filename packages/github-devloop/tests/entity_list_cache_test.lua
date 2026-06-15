local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if tostring(call.rendered or ""):find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

return {
  test_entity_list_cache_key_is_readable_and_buckets_poll_timestamps = function()
    local first = core.entity_list_cache_key("owner/repo", "issue", "open", "2026-06-03T01:02:03Z")
    local second = core.entity_list_cache_key("owner/repo", "issue", "open", "2026-06-03T01:04:59Z")
    local next_bucket = core.entity_list_cache_key("owner/repo", "issue", "open", "2026-06-03T01:05:00Z")

    t.is_true(first:find("^github%-devloop/entity%-list/owner/repo/issue/open/bucket%-%d+$") ~= nil)
    t.eq(first, second)
    t.eq(first == next_bucket, false)
  end,

  test_shared_issue_observe_list_reuses_successful_poll_snapshot = function()
    local repo = "owner/shared-list"
    local command = core.gh_issue_list_observe_cmd(repo)
    t.mock_command(command, {
      stdout = '[{"number":42,"state":"open","updated_at":"2026-06-03T01:02:03Z"}]\n',
      stderr = "",
      exit_code = 0,
    })

    local first = core.fetch_shared_issue_observe_list(repo, {
      poll_key = "2026-06-03T01:02:03Z",
    })
    local second = core.fetch_shared_issue_observe_list(repo, {
      poll_key = "2026-06-03T01:03:03Z",
    })

    t.eq(first.exit_code, 0)
    t.eq(second.exit_code, 0)
    t.eq(second.stdout, first.stdout)
    t.eq(count_calls(command), 1)
  end,

  test_shared_pr_observe_list_failures_are_not_cached = function()
    local repo = "owner/shared-pr-list"
    local command = core.gh_pr_list_observe_cmd(repo)
    t.mock_command(command, {
      stdout = "",
      stderr = "rate limited",
      exit_code = 1,
    })
    t.mock_command(command, {
      stdout = '[{"number":7,"state":"open","updated_at":"2026-06-03T01:02:03Z"}]\n',
      stderr = "",
      exit_code = 0,
    })

    local first = core.fetch_shared_pr_observe_list(repo, {
      poll_key = "2026-06-03T01:02:03Z",
    })
    local second = core.fetch_shared_pr_observe_list(repo, {
      poll_key = "2026-06-03T01:02:03Z",
    })

    t.eq(first.exit_code, 1)
    t.eq(second.exit_code, 0)
    t.eq(count_calls(command), 2)
  end,

  test_shared_intake_list_uses_separate_scope_from_observe_list = function()
    local repo = "owner/shared-intake-list"
    local observe_command = core.gh_issue_list_observe_cmd(repo)
    local intake_command = core.gh_issue_list_intake_cmd(repo, 100)
    t.mock_command(observe_command, {
      stdout = '[{"number":1,"state":"open","updated_at":"2026-06-03T01:02:03Z"}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(intake_command, {
      stdout = '[{"number":2,"title":"Candidate","body":"","updatedAt":"2026-06-03T01:02:03Z","labels":[]}]\n',
      stderr = "",
      exit_code = 0,
    })

    local observe = core.fetch_shared_issue_observe_list(repo, {
      poll_key = "2026-06-03T01:02:03Z",
    })
    local intake = core.fetch_shared_issue_intake_list(repo, 100, {
      poll_key = "2026-06-03T01:02:03Z",
    })

    t.eq(observe.exit_code, 0)
    t.eq(intake.exit_code, 0)
    t.eq(count_calls(observe_command), 1)
    t.eq(count_calls(intake_command), 1)
  end,
}
