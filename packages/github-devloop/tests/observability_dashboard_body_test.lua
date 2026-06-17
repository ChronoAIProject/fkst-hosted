local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local old_dashboard_body_cap = 12000

local function mock_dashboard_env()
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function large_mermaid(line_count)
  local lines = { "flowchart LR" }
  for index = 1, line_count do
    table.insert(lines, "  node_" .. tostring(index) .. " --> node_" .. tostring(index + 1))
  end
  return table.concat(lines, "\n")
end

local function assert_dashboard_marker_outside_fences(body)
  local marker_start = body:find("<!-- fkst:dashboard:v1", 1, true)
  t.is_true(marker_start ~= nil)

  local search_from = 1
  local last_close = nil
  while true do
    local opening = body:find("```mermaid", search_from, true)
    if opening == nil then
      break
    end
    local closing = body:find("\n```", opening + #"```mermaid", true)
    t.is_true(closing ~= nil)
    t.is_true(closing < marker_start)
    t.eq(body:sub(opening, closing):find("<!--", 1, true), nil)
    last_close = closing
    search_from = closing + #"\n```"
  end

  if last_close ~= nil then
    t.is_true(last_close < marker_start)
  end
end

return {
  test_dashboard_renders_large_topology_without_old_cap_cutting_mermaid = function()
    mock_dashboard_env()
    local mermaid = large_mermaid(900)
    t.is_true(#mermaid > old_dashboard_body_cap)

    local dashboard = core.render_observability_dashboard({
      entities = {},
      counts = {},
      stalls = {},
      topology_mermaid = mermaid,
      now_seconds = 1770000000,
    })

    t.is_true(#dashboard.body > old_dashboard_body_cap)
    t.is_true(dashboard.body:find("node_900 --> node_901", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Board by state", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Ready", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Blocked", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Stall suspects", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Footer", 1, true) ~= nil)
    assert_dashboard_marker_outside_fences(dashboard.body)
  end,

  test_dashboard_forced_cap_drops_whole_sections_without_open_fence = function()
    mock_dashboard_env()
    local forced_cap = 2500
    local dashboard = core.render_observability_dashboard({
      entities = {},
      counts = {},
      stalls = {},
      topology_mermaid = large_mermaid(900),
      now_seconds = 1770000000,
      max_body_len = forced_cap,
    })

    t.is_true(#dashboard.body <= forced_cap)
    t.eq(dashboard.body:find("node_900 --> node_901", 1, true), nil)
    assert_dashboard_marker_outside_fences(dashboard.body)
  end,
}
