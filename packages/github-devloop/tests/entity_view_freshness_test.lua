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
}
