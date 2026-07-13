local h = require("tests.devloop_core_helpers")
local restart_edges = require("devloop.restart_edges")

local t = h.t

local function inventory_entry()
  return {
    semantic_variant = "ingress",
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
    semantic_variant = "retry",
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
    semantic_variant = "normalized",
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

local function responsibility_row(successors)
  return {
    from_state = "from",
    responsibility_signature = { successors = successors },
  }
end

local function guard_boundaries_row(successors)
  return {
    from_state = "from",
    guard_boundaries = {
      {
        name = "synthetic_guard",
        successors = successors,
      },
    },
  }
end

return {
  test_entry_cas_metadata_is_optional_and_copied_from_both_declaration_branches = function()
    local with_cas = inventory_entry()
    with_cas.cas_policy_id = "cas.inventory_v1"
    with_cas.cas_variant = "inventory_variant"
    local without_cas = inventory_entry()
    without_cas.semantic_variant = "other"
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

  test_canonicalization_cas_metadata_is_optional_and_copied = function()
    local with_cas = canonicalization_entry()
    with_cas.cas_policy_id = "cas.canonicalization_v1"
    with_cas.cas_variant = "normalized"
    local without_cas = canonicalization_entry()
    without_cas.semantic_variant = "other"
    without_cas.provenance.field = "canonicalization_inventory.other"

    local edges = restart_edges.extract_canonicalization_edges("owner", { with_cas, without_cas })

    t.eq(edges[1].cas_policy_id, "cas.canonicalization_v1")
    t.eq(edges[1].cas_variant, "normalized")
    t.eq(has_key(edges[2], "cas_policy_id"), false)
    t.eq(has_key(edges[2], "cas_variant"), false)
  end,

  test_canonicalization_cas_metadata_fails_closed_on_empty_or_non_string_values = function()
    local invalid_values = {
      { field = "cas_policy_id", value = "" },
      { field = "cas_policy_id", value = 1 },
      { field = "cas_variant", value = "" },
      { field = "cas_variant", value = false },
    }
    for _, invalid in ipairs(invalid_values) do
      local entry = canonicalization_entry()
      entry[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_canonicalization_edges("owner", { entry })
      end, "must be a non-empty string")
    end
  end,

  test_operator_reentry_cas_metadata_is_optional_and_copied = function()
    local with_cas = operator_reentry_entry()
    with_cas.cas_policy_id = "cas.operator_reentry_v1"
    with_cas.cas_variant = "retry"
    local without_cas = operator_reentry_entry()
    without_cas.semantic_variant = "other"
    without_cas.provenance.field = "operator_reentry.other"

    local edges = restart_edges.extract_operator_reentry_edges("owner", { with_cas, without_cas })

    t.eq(edges[1].cas_policy_id, "cas.operator_reentry_v1")
    t.eq(edges[1].cas_variant, "retry")
    t.eq(has_key(edges[2], "cas_policy_id"), false)
    t.eq(has_key(edges[2], "cas_variant"), false)
  end,

  test_operator_reentry_cas_metadata_fails_closed_on_empty_or_non_string_values = function()
    local invalid_values = {
      { field = "cas_policy_id", value = "" },
      { field = "cas_policy_id", value = 1 },
      { field = "cas_variant", value = "" },
      { field = "cas_variant", value = false },
    }
    for _, invalid in ipairs(invalid_values) do
      local entry = operator_reentry_entry()
      entry[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_operator_reentry_edges("owner", { entry })
      end, "must be a non-empty string")
    end
  end,

  test_guard_boundary_cas_metadata_is_optional_and_copied_from_both_declaration_branches = function()
    local responsibility = responsibility_row({
      {
        state = "with-cas",
        output_variant = "responsibility-with-cas",
        kind = "guard_boundary",
        cas_policy_id = "cas.responsibility_v1",
        cas_variant = "responsibility_variant",
      },
      { state = "without-cas", output_variant = "responsibility-without-cas", kind = "guard_boundary" },
    })
    local guard_boundaries = guard_boundaries_row({
      {
        state = "with-cas",
        output_variant = "array-with-cas",
        kind = "guard_boundary",
        cas_policy_id = "cas.array_v1",
        cas_variant = "array_variant",
      },
      { state = "without-cas", output_variant = "array-without-cas" },
    })

    local edges = restart_edges.extract_guard_boundary_edges("owner", { responsibility, guard_boundaries })

    t.eq(edges[1].cas_policy_id, "cas.responsibility_v1")
    t.eq(edges[1].cas_variant, "responsibility_variant")
    t.eq(has_key(edges[2], "cas_policy_id"), false)
    t.eq(has_key(edges[2], "cas_variant"), false)
    t.eq(edges[3].cas_policy_id, "cas.array_v1")
    t.eq(edges[3].cas_variant, "array_variant")
    t.eq(has_key(edges[4], "cas_policy_id"), false)
    t.eq(has_key(edges[4], "cas_variant"), false)
    t.eq(#restart_edges.extract_timeout_edges("owner", { responsibility, guard_boundaries }), 0)
  end,

  test_guard_boundary_cas_metadata_fails_closed_on_empty_or_non_string_values_in_both_branches = function()
    local invalid_values = {
      { field = "cas_policy_id", value = "" },
      { field = "cas_policy_id", value = 1 },
      { field = "cas_variant", value = "" },
      { field = "cas_variant", value = false },
    }
    for _, invalid in ipairs(invalid_values) do
      local responsibility_successor = {
        state = "to",
        output_variant = "responsibility-invalid",
        kind = "guard_boundary",
      }
      responsibility_successor[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_guard_boundary_edges("owner", {
          responsibility_row({ responsibility_successor }),
        })
      end, "must be a non-empty string")

      local array_successor = {
        state = "to",
        output_variant = "array-invalid",
        kind = "guard_boundary",
      }
      array_successor[invalid.field] = invalid.value
      assert_error_contains(function()
        restart_edges.extract_guard_boundary_edges("owner", {
          guard_boundaries_row({ array_successor }),
        })
      end, "must be a non-empty string")
    end
  end,

  test_timeout_cas_metadata_is_optional_and_copied_from_both_declaration_branches = function()
    local responsibility = responsibility_row({
      {
        state = "with-cas",
        output_variant = "responsibility-with-cas",
        kind = "timeout",
        cas_policy_id = "cas.responsibility_timeout_v1",
        cas_variant = "responsibility_variant",
      },
      { state = "without-cas", output_variant = "responsibility-without-cas", kind = "timeout" },
    })
    responsibility.actionable_epoch = { source = "state_entry:v1" }
    local guard_boundaries = guard_boundaries_row({
      {
        state = "with-cas",
        output_variant = "array-with-cas",
        kind = "timeout",
        cas_policy_id = "cas.array_timeout_v1",
        cas_variant = "array_variant",
      },
      { state = "without-cas", output_variant = "array-without-cas", kind = "timeout" },
    })
    guard_boundaries.actionable_epoch = { source = "live_defer_heartbeat:v1" }

    local edges = restart_edges.extract_timeout_edges("owner", { responsibility, guard_boundaries })

    t.eq(edges[1].timeout_evidence_policy_id, "timeout.state_entry_legacy_v1")
    t.eq(edges[1].cas_policy_id, "cas.responsibility_timeout_v1")
    t.eq(edges[1].cas_variant, "responsibility_variant")
    t.eq(edges[2].timeout_evidence_policy_id, "timeout.state_entry_legacy_v1")
    t.eq(has_key(edges[2], "cas_policy_id"), false)
    t.eq(has_key(edges[2], "cas_variant"), false)
    t.eq(edges[3].timeout_evidence_policy_id, "timeout.heartbeat_legacy_v1")
    t.eq(edges[3].cas_policy_id, "cas.array_timeout_v1")
    t.eq(edges[3].cas_variant, "array_variant")
    t.eq(edges[4].timeout_evidence_policy_id, "timeout.heartbeat_legacy_v1")
    t.eq(has_key(edges[4], "cas_policy_id"), false)
    t.eq(has_key(edges[4], "cas_variant"), false)
    t.eq(#restart_edges.extract_guard_boundary_edges("owner", { responsibility, guard_boundaries }), 0)
  end,

  test_timeout_cas_metadata_fails_closed_on_empty_or_non_string_values_in_both_branches = function()
    local invalid_values = {
      { field = "cas_policy_id", value = "" },
      { field = "cas_policy_id", value = 1 },
      { field = "cas_variant", value = "" },
      { field = "cas_variant", value = false },
    }
    for _, invalid in ipairs(invalid_values) do
      local responsibility_successor = {
        state = "to",
        output_variant = "responsibility-invalid",
        kind = "timeout",
      }
      responsibility_successor[invalid.field] = invalid.value
      local responsibility = responsibility_row({ responsibility_successor })
      responsibility.actionable_epoch = { source = "state_entry:v1" }
      assert_error_contains(function()
        restart_edges.extract_timeout_edges("owner", { responsibility })
      end, "must be a non-empty string")

      local array_successor = {
        state = "to",
        output_variant = "array-invalid",
        kind = "timeout",
      }
      array_successor[invalid.field] = invalid.value
      local guard_boundaries = guard_boundaries_row({ array_successor })
      guard_boundaries.actionable_epoch = { source = "state_entry:v1" }
      assert_error_contains(function()
        restart_edges.extract_timeout_edges("owner", { guard_boundaries })
      end, "must be a non-empty string")
    end
  end,
}
