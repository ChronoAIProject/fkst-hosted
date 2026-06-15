local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function issue_view_stdout(title)
  return '{"title":"' .. tostring(title) .. '","body":"","state":"OPEN","labels":[],"comments":[],"assignees":[],"updatedAt":"2026-06-03T01:02:03Z"}\n'
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
  test_marker_bearing_issue_state_reader_accepts_explicit_timeout = function()
    local seen_timeout = nil
    local old_gh_exec = core.gh_exec
    core.gh_exec = function(cmd_or_opts, timeout)
      if type(cmd_or_opts) == "table" then
        seen_timeout = cmd_or_opts.timeout
      else
        seen_timeout = timeout
      end
      return {
        stdout = issue_view_stdout("Timeout"),
        stderr = "",
        exit_code = 0,
      }
    end

    local ok, result = pcall(function()
      return core.fetch_issue_view_state("owner/repo", 42, "2026-06-03T01:02:03Z", {
        timeout = 10,
      })
    end)
    core.gh_exec = old_gh_exec

    t.eq(ok, true, tostring(result))
    t.eq(result.exit_code, 0)
    t.eq(seen_timeout, 10)
  end,

  test_post_write_invalidation_bypasses_same_updated_at_issue_view_cache = function()
    local repo = "owner/cache-invalidation"
    local issue_number = 4242
    local command = core.gh_issue_view_entity_cmd(repo, issue_number)
    local updated_at_command = core.gh_entity_updated_at_cmd(repo, "issue", issue_number)
    local generation_key = core.entity_view_generation_key(repo, "issue", issue_number)
    local initial_generation = tonumber(cache_get(generation_key) or "0") or 0
    t.mock_command(command, {
      stdout = issue_view_stdout("Before"),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(updated_at_command, {
      stdout = "2026-06-03T01:02:03Z\n",
      stderr = "",
      exit_code = 0,
    })
    local cached = core.fetch_issue_view(repo, issue_number, "2026-06-03T01:02:03Z", {
      consumer = "first-reader",
    })
    t.eq(cached.exit_code, 0)
    t.is_true(cached.stdout:find('"Before"', 1, true) ~= nil)

    core.invalidate_entity_after_write(repo, "issue", issue_number)
    t.eq(tonumber(cache_get(generation_key)), initial_generation + 1)

    t.mock_command(command, {
      stdout = issue_view_stdout("After"),
      stderr = "",
      exit_code = 0,
    })
    local after = core.fetch_issue_view(repo, issue_number, "2026-06-03T01:02:03Z", {
      consumer = "second-reader",
    })
    t.eq(after.exit_code, 0)
    t.eq(count_calls(command), 2)
  end,

  test_marker_bearing_issue_state_reader_bypasses_entity_view_cache = function()
    local command = core.gh_issue_view_entity_cmd("owner/repo", 42)
    t.mock_command(command, {
      stdout = issue_view_stdout("Before"),
      stderr = "",
      exit_code = 0,
    })
    local cached = core.fetch_issue_view("owner/repo", 42, "2026-06-03T01:02:03Z", {
      consumer = "non-marker-reader",
    })
    t.eq(cached.exit_code, 0)
    t.is_true(cached.stdout:find('"Before"', 1, true) ~= nil)

    t.mock_command(command, {
      stdout = issue_view_stdout("After"),
      stderr = "",
      exit_code = 0,
    })
    local state = core.fetch_issue_view_state("owner/repo", 42, "2026-06-03T01:02:03Z")
    t.eq(state.exit_code, 0)
    t.is_true(state.stdout:find('"After"', 1, true) ~= nil)
    t.eq(count_calls(command), 2)
  end,

  test_marker_bearing_pr_origin_reader_bypasses_entity_view_cache = function()
    local command = core.gh_pr_view_entity_cmd("owner/repo", 7)
    t.mock_command(command, {
      stdout = '{"headRefName":"branch","headRefOid":"abc123","baseRefName":"dev","state":"OPEN","comments":[],"updatedAt":"2026-06-03T01:02:03Z"}\n',
      stderr = "",
      exit_code = 0,
    })
    local cached = core.fetch_pr_view("owner/repo", 7, "2026-06-03T01:02:03Z", {
      consumer = "non-marker-reader",
    })
    t.eq(cached.exit_code, 0)
    t.is_true(cached.stdout:find('"abc123"', 1, true) ~= nil)

    t.mock_command(command, {
      stdout = '{"headRefName":"branch","headRefOid":"def456","baseRefName":"dev","state":"OPEN","comments":[],"updatedAt":"2026-06-03T01:02:03Z"}\n',
      stderr = "",
      exit_code = 0,
    })
    local origin = core.fetch_pr_view_origin("owner/repo", 7, "2026-06-03T01:02:03Z")
    t.eq(origin.exit_code, 0)
    t.is_true(origin.stdout:find('"def456"', 1, true) ~= nil)
    t.eq(count_calls(command), 2)
  end,
}
