local h = require("tests.devloop_core_helpers")
local restart_edges = require("devloop.restart_edges")

local t = h.t

local function row(successors)
  return {
    from_state = "thinking",
    responsibility_signature = { successors = successors },
  }
end

local function successor(output_variant)
  return {
    state = "ready",
    output_variant = output_variant,
    kind = "autonomous",
  }
end

local function has_key(value, expected_key)
  for key in pairs(value) do
    if key == expected_key then
      return true
    end
  end
  return false
end

local function assert_extract_error(authored)
  local ok = pcall(function()
    restart_edges.extract_autonomous_edges("owner", { row({ authored }) })
  end)
  t.eq(ok, false)
end

return {
  test_autonomous_pending_order_is_optional_and_copied_verbatim = function()
    local with_pending = successor("with-pending")
    with_pending.pending_order = {
      participates = true,
      predecessor_state = "thinking",
    }
    local without_pending = successor("without-pending")

    local edges = restart_edges.extract_autonomous_edges(
      "owner",
      { row({ with_pending, without_pending }) }
    )

    t.eq(#edges, 2)
    t.is_true(edges[1].pending_order == with_pending.pending_order)
    t.eq(edges[1].pending_order.participates, true)
    t.eq(edges[1].pending_order.predecessor_state, "thinking")
    t.eq(has_key(edges[2], "pending_order"), false)
  end,

  test_autonomous_pending_order_requires_boolean_participation = function()
    local authored = successor("invalid-participation")
    authored.pending_order = {
      participates = "yes",
      predecessor_state = "thinking",
    }

    assert_extract_error(authored)
  end,

  test_participating_pending_order_requires_predecessor_state = function()
    local authored = successor("missing-predecessor")
    authored.pending_order = { participates = true }

    assert_extract_error(authored)
  end,
}
