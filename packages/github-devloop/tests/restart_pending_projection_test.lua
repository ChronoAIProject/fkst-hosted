local h = require("tests.devloop_core_helpers")
local pending_projection = require("devloop.restart_pending_projection")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local function thinking_ready_edge()
  local expected_id = core.restart_package_name .. "/thinking/autonomous/consensus-reached"
  for _, edge in ipairs(restart_edges.extract_autonomous_edges(
    core.restart_package_name,
    core.restart_transition_table()
  )) do
    if edge.id == expected_id then
      return edge
    end
  end
  error("annotated thinking-to-ready edge not found")
end

local function assert_error(fn)
  local ok = pcall(fn)
  t.eq(ok, false)
end

return {
  test_pending_projection_derives_unique_pairs_and_reachability = function()
    local projection = pending_projection.derive_pending_projection({
      thinking_ready_edge(),
      {
        target = "blocked",
        pending_order = {
          participates = true,
          predecessor_state = "ready",
        },
      },
      {
        target = "blocked",
        pending_order = {
          participates = true,
          predecessor_state = "ready",
        },
      },
      {
        target = "merged",
        pending_order = { participates = false },
      },
      {
        -- Well-formed non-participating edge that DOES carry a predecessor_state:
        -- it must be excluded from the projection (participates-gating, not presence).
        target = "declined",
        pending_order = { participates = false, predecessor_state = "thinking" },
      },
    })

    t.eq(projection.thinking.ready, true)
    t.eq(projection.ready.blocked, true)
    -- positive exclusion: the well-formed non-participating thinking->declined pair does NOT leak.
    t.eq(projection.thinking.declined, nil)
    t.eq(pending_projection.can_reach(projection, "thinking", "ready"), true)
    t.eq(pending_projection.can_reach(projection, "thinking", "blocked"), true)
    t.eq(pending_projection.can_reach(projection, "thinking", "declined"), false)
    t.eq(pending_projection.can_reach(projection, "ready", "thinking"), false)
    t.eq(pending_projection.can_reach(projection, nil, "unmanaged"), true)
  end,

  test_pending_projection_requires_explicit_pending_order = function()
    assert_error(function()
      pending_projection.derive_pending_projection({ { target = "ready" } })
    end)
  end,

  test_pending_projection_requires_participating_predecessor_and_lifecycle_target = function()
    assert_error(function()
      pending_projection.derive_pending_projection({
        { target = "ready", pending_order = { participates = true } },
      })
    end)
    assert_error(function()
      pending_projection.derive_pending_projection({
        {
          target = "",
          pending_order = {
            participates = true,
            predecessor_state = "thinking",
          },
        },
      })
    end)
  end,
}
