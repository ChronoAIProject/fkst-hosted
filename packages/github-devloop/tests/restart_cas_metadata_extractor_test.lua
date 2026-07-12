local h = require("tests.devloop_core_helpers")
local restart_edges = require("devloop.restart_edges")

local t = h.t

local function inventory_entry()
  return {
    id = "owner/thinking/entry/ingress",
    owner = "owner",
    row_id = "thinking",
    kind = "entry",
    source = { state = nil, boundary = "owner.ingress" },
    target = "thinking",
    provenance = {
      owner = "owner",
      row = "thinking",
      field = "entry_inventory.ingress",
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

local function has_key(value, expected_key)
  for key in pairs(value) do
    if key == expected_key then
      return true
    end
  end
  return false
end

local function assert_error_contains(fn, expected)
  local ok, err = pcall(fn)
  t.eq(ok, false)
  t.is_true(tostring(err):find(expected, 1, true) ~= nil)
end

local function operator_reentry_entry()
  return {
    id = "owner/blocked/operator_reentry/retry",
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
      field = "operator_reentry",
    },
  }
end

local function canonicalization_entry()
  return {
    id = "owner/reviewing/canonicalization/normalized",
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
      field = "canonicalization_inventory.normalized",
    },
  }
end

return {
  test_entry_cas_metadata_is_optional_and_copied_from_both_declaration_branches = function()
    local with_cas = inventory_entry()
    with_cas.cas_policy_id = "cas.inventory_v1"
    with_cas.cas_variant = "inventory_variant"
    local without_cas = inventory_entry()
    without_cas.id = "owner/thinking/entry/other"
    without_cas.provenance.field = "entry_inventory.other"

    local receiver_with_cas = activation("receiver_with_cas")
    receiver_with_cas.cas_policy_id = "cas.receiver_v1"
    receiver_with_cas.cas_variant = "receiver_variant"
    local receiver_without_cas = activation("receiver_without_cas")
    local edges = restart_edges.extract_entry_edges("owner", { with_cas, without_cas }, {
      activation_row({ receiver_with_cas, receiver_without_cas }),
    })

    t.eq(edges[1].cas_policy_id, "cas.inventory_v1")
    t.eq(edges[1].cas_variant, "inventory_variant")
    t.eq(has_key(edges[2], "cas_policy_id"), false)
    t.eq(has_key(edges[2], "cas_variant"), false)
    t.eq(edges[3].cas_policy_id, "cas.receiver_v1")
    t.eq(edges[3].cas_variant, "receiver_variant")
    t.eq(has_key(edges[4], "cas_policy_id"), false)
    t.eq(has_key(edges[4], "cas_variant"), false)
  end,

  test_entry_cas_metadata_fails_closed_on_empty_or_non_string_values_in_both_branches = function()
    local invalid_values = {
      { field = "cas_policy_id", value = "" },
      { field = "cas_policy_id", value = 1 },
      { field = "cas_variant", value = "" },
      { field = "cas_variant", value = false },
    }
    for _, invalid in ipairs(invalid_values) do
      local entry = inventory_entry()
      entry[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_entry_edges("owner", { entry }, {})
      end, "must be a non-empty string")

      local receiver = activation("invalid")
      receiver[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_entry_edges("owner", {}, { activation_row({ receiver }) })
      end, "must be a non-empty string")
    end
  end,

  test_cas_metadata_fails_closed_on_unsupported_edge_kinds = function()
    local guard_row = {
      from_state = "from",
      responsibility_signature = {
        successors = {
          {
            state = "to",
            output_variant = "guarded",
            kind = "guard_boundary",
            cas_policy_id = "cas.unsupported_v1",
          },
        },
      },
    }
    assert_error_contains(function()
      restart_edges.extract_guard_boundary_edges("owner", { guard_row })
    end, "cas_policy_id/cas_variant not supported on guard_boundary edges")

    local timeout_row = {
      from_state = "from",
      actionable_epoch = { source = "state_entry:v1" },
      responsibility_signature = {
        successors = {
          {
            state = "to",
            output_variant = "timed_out",
            kind = "timeout",
            cas_variant = "unsupported",
          },
        },
      },
    }
    assert_error_contains(function()
      restart_edges.extract_timeout_edges("owner", { timeout_row })
    end, "cas_policy_id/cas_variant not supported on timeout edges")

    local operator_entry = operator_reentry_entry()
    operator_entry.cas_policy_id = "cas.unsupported_v1"
    assert_error_contains(function()
      restart_edges.extract_operator_reentry_edges("owner", { operator_entry })
    end, "cas_policy_id/cas_variant not supported on operator_reentry edges")

    local canonical_entry = canonicalization_entry()
    canonical_entry.cas_variant = "unsupported"
    assert_error_contains(function()
      restart_edges.extract_canonicalization_edges("owner", { canonical_entry })
    end, "cas_policy_id/cas_variant not supported on canonicalization edges")
  end,
}
