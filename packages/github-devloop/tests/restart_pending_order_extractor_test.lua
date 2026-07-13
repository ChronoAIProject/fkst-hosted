local h = require("tests.devloop_core_helpers")
local restart_edges = require("devloop.restart_edges")

local t = h.t

local function pending_order()
  return {
    participates = true,
    predecessor_state = "thinking",
  }
end

local function with_pending(declaration)
  declaration.pending_order = pending_order()
  return declaration
end

local function without_pending(declaration)
  return declaration
end

local function invalidate_pending(declaration)
  declaration.pending_order = {
    participates = "yes",
    predecessor_state = "thinking",
  }
  return declaration
end

local function has_key(value, expected_key)
  for key in pairs(value) do
    if key == expected_key then
      return true
    end
  end
  return false
end

local function assert_optional_copy(edge_with_pending, declaration, edge_without_pending)
  t.is_true(edge_with_pending.pending_order == declaration.pending_order)
  t.eq(edge_with_pending.pending_order.participates, true)
  t.eq(edge_with_pending.pending_order.predecessor_state, "thinking")
  t.eq(has_key(edge_without_pending, "pending_order"), false)
end

local function assert_extract_error(fn)
  local ok = pcall(fn)
  t.eq(ok, false)
end

local function responsibility_row(successors)
  return {
    from_state = "thinking",
    responsibility_signature = { successors = successors },
  }
end

local function successor(kind, output_variant)
  return {
    state = "ready",
    output_variant = output_variant,
    kind = kind,
  }
end

local function inventory_entry(output_variant)
  return {
    id = "owner/thinking/entry/" .. output_variant,
    owner = "owner",
    row_id = "thinking",
    kind = "entry",
    source = { state = nil, boundary = "owner.ingress" },
    target = "thinking",
    provenance = {
      owner = "owner",
      row = "thinking",
      field = "entry_inventory." .. output_variant,
    },
  }
end

local function activation(output_variant)
  return {
    kind = "entry",
    boundary = "owner.receiver",
    target = "blocked",
    output_variant = output_variant,
  }
end

local function activation_row(activations)
  return {
    from_state = "thinking",
    receiver_activations = activations,
  }
end

local function operator_reentry_entry(output_variant)
  return {
    id = "owner/blocked/operator_reentry/" .. output_variant,
    owner = "owner",
    row_id = "blocked",
    kind = "operator_reentry",
    source = { state = "blocked", boundary = nil },
    target = "thinking",
    cause_evidence = {
      command = "retry",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "owner",
      row = "blocked",
      field = "operator_reentry." .. output_variant,
    },
  }
end

local function canonicalization_entry(output_variant)
  return {
    id = "owner/reviewing/canonicalization/" .. output_variant,
    owner = "owner",
    row_id = "reviewing",
    kind = "canonicalization",
    source = { state = "reviewing", boundary = nil },
    target = "reviewing",
    cause_evidence = {
      marker = "state:v1",
      resolver = "state_marker",
    },
    provenance = {
      owner = "owner",
      row = "reviewing",
      field = "canonicalization_inventory." .. output_variant,
    },
  }
end

local function guard_boundaries_row(successors)
  return {
    from_state = "thinking",
    guard_boundaries = {
      {
        name = "synthetic_guard",
        successors = successors,
      },
    },
  }
end

return {
  test_autonomous_pending_order_is_optional_and_copied_verbatim = function()
    local with_declaration = with_pending(successor("autonomous", "with-pending"))
    local without_declaration = without_pending(successor("autonomous", "without-pending"))

    local edges = restart_edges.extract_autonomous_edges(
      "owner",
      { responsibility_row({ with_declaration, without_declaration }) }
    )

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], with_declaration, edges[2])
  end,

  test_autonomous_pending_order_requires_boolean_participation = function()
    local authored = invalidate_pending(successor("autonomous", "invalid-participation"))

    assert_extract_error(function()
      restart_edges.extract_autonomous_edges("owner", { responsibility_row({ authored }) })
    end)
  end,

  test_participating_pending_order_requires_predecessor_state = function()
    local authored = successor("autonomous", "missing-predecessor")
    authored.pending_order = { participates = true }

    assert_extract_error(function()
      restart_edges.extract_autonomous_edges("owner", { responsibility_row({ authored }) })
    end)
  end,

  test_entry_inventory_pending_order_is_optional_and_copied = function()
    local inventory_with = with_pending(inventory_entry("inventory-with"))
    local inventory_without = without_pending(inventory_entry("inventory-without"))

    local edges = restart_edges.extract_entry_edges(
      "owner",
      { inventory_with, inventory_without },
      {}
    )

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], inventory_with, edges[2])
  end,

  test_entry_inventory_pending_order_fails_closed = function()
    local inventory = invalidate_pending(inventory_entry("inventory-invalid"))
    assert_extract_error(function()
      restart_edges.extract_entry_edges("owner", { inventory }, {})
    end)
  end,

  test_receiver_activation_pending_order_is_optional_and_copied = function()
    local receiver_with = with_pending(activation("receiver-with"))
    local receiver_without = without_pending(activation("receiver-without"))

    local edges = restart_edges.extract_entry_edges(
      "owner",
      {},
      { activation_row({ receiver_with, receiver_without }) }
    )

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], receiver_with, edges[2])
  end,

  test_receiver_activation_pending_order_fails_closed = function()
    local receiver = invalidate_pending(activation("receiver-invalid"))
    assert_extract_error(function()
      restart_edges.extract_entry_edges("owner", {}, { activation_row({ receiver }) })
    end)
  end,

  test_operator_reentry_pending_order_is_optional_and_copied = function()
    local with_declaration = with_pending(operator_reentry_entry("with-pending"))
    local without_declaration = without_pending(operator_reentry_entry("without-pending"))

    local edges = restart_edges.extract_operator_reentry_edges(
      "owner",
      { with_declaration, without_declaration }
    )

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], with_declaration, edges[2])
  end,

  test_operator_reentry_pending_order_fails_closed = function()
    local authored = invalidate_pending(operator_reentry_entry("invalid"))
    assert_extract_error(function()
      restart_edges.extract_operator_reentry_edges("owner", { authored })
    end)
  end,

  test_canonicalization_pending_order_is_optional_and_copied = function()
    local with_declaration = with_pending(canonicalization_entry("with-pending"))
    local without_declaration = without_pending(canonicalization_entry("without-pending"))

    local edges = restart_edges.extract_canonicalization_edges(
      "owner",
      { with_declaration, without_declaration }
    )

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], with_declaration, edges[2])
  end,

  test_canonicalization_pending_order_fails_closed = function()
    local authored = invalidate_pending(canonicalization_entry("invalid"))
    assert_extract_error(function()
      restart_edges.extract_canonicalization_edges("owner", { authored })
    end)
  end,

  test_responsibility_guard_pending_order_is_optional_and_copied = function()
    local responsibility_with = with_pending(successor("guard_boundary", "responsibility-with"))
    local responsibility_without = without_pending(successor("guard_boundary", "responsibility-without"))

    local edges = restart_edges.extract_guard_boundary_edges("owner", {
      responsibility_row({ responsibility_with, responsibility_without }),
    })

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], responsibility_with, edges[2])
  end,

  test_responsibility_guard_pending_order_fails_closed = function()
    local responsibility = invalidate_pending(successor("guard_boundary", "responsibility-invalid"))
    assert_extract_error(function()
      restart_edges.extract_guard_boundary_edges("owner", {
        responsibility_row({ responsibility }),
      })
    end)
  end,

  test_guard_boundary_array_pending_order_is_optional_and_copied = function()
    local array_with = with_pending(successor("guard_boundary", "array-with"))
    local array_without = without_pending(successor(nil, "array-without"))

    local edges = restart_edges.extract_guard_boundary_edges("owner", {
      guard_boundaries_row({ array_with, array_without }),
    })

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], array_with, edges[2])
  end,

  test_guard_boundary_array_pending_order_fails_closed = function()
    local array = invalidate_pending(successor("guard_boundary", "array-invalid"))
    assert_extract_error(function()
      restart_edges.extract_guard_boundary_edges("owner", {
        guard_boundaries_row({ array }),
      })
    end)
  end,

  test_responsibility_timeout_pending_order_is_optional_and_copied = function()
    local responsibility_with = with_pending(successor("timeout", "responsibility-with"))
    local responsibility_without = without_pending(successor("timeout", "responsibility-without"))
    local responsibility = responsibility_row({ responsibility_with, responsibility_without })
    responsibility.actionable_epoch = { source = "state_entry:v1" }

    local edges = restart_edges.extract_timeout_edges("owner", { responsibility })

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], responsibility_with, edges[2])
  end,

  test_responsibility_timeout_pending_order_fails_closed = function()
    local responsibility_successor = invalidate_pending(successor("timeout", "responsibility-invalid"))
    local responsibility = responsibility_row({ responsibility_successor })
    responsibility.actionable_epoch = { source = "state_entry:v1" }
    assert_extract_error(function()
      restart_edges.extract_timeout_edges("owner", { responsibility })
    end)
  end,

  test_guard_boundary_timeout_pending_order_is_optional_and_copied = function()
    local array_with = with_pending(successor("timeout", "array-with"))
    local array_without = without_pending(successor("timeout", "array-without"))
    local guard_boundaries = guard_boundaries_row({ array_with, array_without })
    guard_boundaries.actionable_epoch = { source = "live_defer_heartbeat:v1" }

    local edges = restart_edges.extract_timeout_edges("owner", { guard_boundaries })

    t.eq(#edges, 2)
    assert_optional_copy(edges[1], array_with, edges[2])
  end,

  test_guard_boundary_timeout_pending_order_fails_closed = function()
    local array_successor = invalidate_pending(successor("timeout", "array-invalid"))
    local guard_boundaries = guard_boundaries_row({ array_successor })
    guard_boundaries.actionable_epoch = { source = "state_entry:v1" }
    assert_extract_error(function()
      restart_edges.extract_timeout_edges("owner", { guard_boundaries })
    end)
  end,
}
