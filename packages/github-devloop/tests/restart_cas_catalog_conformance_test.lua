local transition_version = require("contract.transition_version")
local catalog = require("devloop.restart_cas_catalog")
local devloop_state = require("devloop.state")
local h = require("tests.devloop_helpers")

local t = h.t

local BASE = "ready/cas-parity"
local V_OLDER = BASE .. "/loop/1"
local V_EQUAL = BASE .. "/loop/2"
local V_NEWER = BASE .. "/loop/3"
local V_OTHER = "ready/cas-parity-other/loop/2"

local complete_policy_ids = {
  "cas.legacy_loop_plain_v1",
  "cas.legacy_consensus_result_v1",
  "cas.legacy_issue_reconcile_v1",
  "cas.legacy_timeout_reconcile_v1",
  "cas.legacy_observe_issue_entry_v1",
  "cas.legacy_awaiting_pr_v1",
  "cas.legacy_observe_pr_v1",
  "cas.legacy_review_result_v1",
  "cas.legacy_fix_v1",
  "cas.legacy_review_meta_v1",
  "cas.legacy_merge_v1",
  "cas.legacy_pr_fix_reconcile_v1",
  "cas.legacy_review_loop_safe_v1",
  "cas.legacy_review_activation_handoff_v1",
  "cas.legacy_implement_activation_handoff_v1",
}

local base_policy_ids = {
  "cas.base_plain_legacy_v1",
  "cas.base_versioned_legacy_v1",
  "cas.base_cyclic_legacy_v1",
}

local function result(status, reason_code, cas_outcome)
  return {
    status = status,
    reason_code = reason_code,
    cas_outcome = cas_outcome,
  }
end

local function assert_result(actual, expected, label)
  t.eq(type(actual), "table")
  if actual.status ~= expected.status
    or actual.reason_code ~= expected.reason_code
    or actual.cas_outcome ~= expected.cas_outcome then
    error("CAS parity mismatch " .. tostring(label or "unlabelled")
      .. ": expected=" .. tostring(expected.status) .. "/" .. tostring(expected.reason_code) .. "/" .. tostring(expected.cas_outcome)
      .. " actual=" .. tostring(actual.status) .. "/" .. tostring(actual.reason_code) .. "/" .. tostring(actual.cas_outcome))
  end
  t.eq(actual.status, expected.status)
  t.eq(actual.reason_code, expected.reason_code)
  t.eq(actual.cas_outcome, expected.cas_outcome)
end

local function production_result(current, status, incoming_version)
  if status == "apply" then
    return result("apply", "apply", "applied")
  end
  if status == "idempotent" then
    return result("idempotent", "already-at-target", "skip-idempotent(already at to_state)")
  end
  if status == "pending" then
    return result("pending", "source-marker-not-visible", "retry-pending(from-state marker not yet visible)")
  end
  if status == "stale" then
    local outcome = devloop_state.cas_outcome(current, status, incoming_version)
    local reason = outcome == "skip-stale(incoming version < current marker version)"
      and "incoming-version-older"
      or "advanced-or-diverged"
    return result("stale", reason, outcome)
  end
  error("unexpected production status: " .. tostring(status))
end

local function raw_overlay(expected, current, source_states, compared_version, statuses)
  if statuses[expected.status] ~= true then
    return expected
  end
  local source = false
  for _, state_name in ipairs(source_states) do
    if tostring(current and current.state or "") == tostring(state_name) then
      source = true
    end
  end
  if not source then
    return result("stale", "from-state-mismatch", "skip-stale(from-state-mismatch)")
  end
  if tostring(current and current.version or "") ~= tostring(compared_version or "") then
    return result("stale", "version-mismatch", "skip-stale(version-mismatch)")
  end
  return expected
end

local function safe_overlay(expected, current, source_states, compared_version, statuses)
  if statuses[expected.status] ~= true then
    return expected
  end
  local source = false
  for _, state_name in ipairs(source_states) do
    if tostring(current and current.state or "") == tostring(state_name) then
      source = true
    end
  end
  if not source then
    return result("stale", "from-state-mismatch", "skip-stale(from-state-mismatch)")
  end
  if transition_version.safe_version_segment(current and current.version)
    ~= transition_version.safe_version_segment(compared_version) then
    return result("stale", "version-mismatch", "skip-stale(version-mismatch)")
  end
  return expected
end

local function production_versioned(current, sources, target, incoming)
  return production_result(
    current,
    devloop_state.versioned_transition_status(current, sources, target, incoming),
    incoming
  )
end

local function production_cyclic(current, sources, target, incoming, target_version)
  return production_result(
    current,
    devloop_state.cyclic_transition_status(current, sources, target, incoming, target_version),
    incoming
  )
end

local function evidence(current, variant, incoming_version, extras)
  local out = {
    current = current,
    variant = variant,
    incoming_version = incoming_version,
  }
  for key, value in pairs(extras or {}) do
    out[key] = value
  end
  return out
end

local function assert_base_matrix(policy_id, production, sources, target, target_version)
  local currents = {
    source = { state = sources[1], version = V_EQUAL },
    target = { state = target, version = target_version or V_EQUAL },
    predecessor = { state = "unmanaged", version = V_EQUAL },
    missing = { state = nil, version = nil },
    unrelated = { state = "merged", version = V_EQUAL },
  }
  local versions = {
    { name = "older", value = V_OLDER },
    { name = "equal", value = V_EQUAL },
    { name = "newer", value = V_NEWER },
    { name = "missing", value = nil },
  }
  for _, current in pairs(currents) do
    for _, version_case in ipairs(versions) do
      local incoming = version_case.value
      local expected = production(current, sources, target, incoming, target_version)
      local actual = catalog.resolve(policy_id, {
        current = current,
        source_states = sources,
        target_state = target,
        incoming_version = incoming,
        target_version = target_version,
      })
      assert_result(actual, expected, policy_id .. "/" .. tostring(current.state) .. "/" .. version_case.name)
    end
  end
end

local function binding_set()
  local seen = {}
  for _, binding in ipairs(catalog.bindings()) do
    t.eq(type(binding.id), "string")
    t.eq(type(binding.consumer), "string")
    t.eq(type(binding.cas_policy_id), "string")
    local definition = catalog.definition(binding.cas_policy_id)
    t.is_true(definition ~= nil)
    if binding.variant ~= nil and definition.variants ~= nil then
      t.is_true(definition.variants[binding.variant] ~= nil)
    end
    t.eq(seen[binding.id], nil)
    seen[binding.id] = true
  end
  return seen
end

local function profile_production_result(profile, current, incoming)
  local current_for_base = current
  local incoming_for_base = incoming
  if profile.base_current_version_form == "safe" then
    current_for_base = {
      state = current.state,
      version = transition_version.safe_version_segment(current.version),
    }
    incoming_for_base = transition_version.safe_version_segment(incoming)
  end
  local expected
  if profile.base == "plain" then
    expected = production_result(
      current_for_base,
      devloop_state.transition_status(current_for_base, profile.sources, profile.target),
      nil
    )
  elseif profile.base == "versioned" then
    expected = production_versioned(current_for_base, profile.sources, profile.target, incoming_for_base)
  else
    expected = production_cyclic(
      current_for_base,
      profile.sources,
      profile.target,
      incoming_for_base,
      profile.target_version
    )
  end
  local overlay_applies = profile.overlay_only_states == nil
  if profile.overlay_only_states ~= nil then
    for _, state_name in ipairs(profile.overlay_only_states) do
      if current.state == state_name then overlay_applies = true end
    end
  end
  local overlay_version = profile.overlay_version or incoming
  if overlay_applies and profile.overlay == "raw" then
    expected = raw_overlay(expected, current, profile.overlay_sources or profile.sources, overlay_version, profile.overlay_statuses)
  elseif overlay_applies and profile.overlay == "safe" then
    expected = safe_overlay(expected, current, profile.overlay_sources or profile.sources, overlay_version, profile.overlay_statuses)
  end
  return expected, incoming_for_base
end

local function assert_complete_profile_matrix(profile)
  local current_versions = {
    source = V_EQUAL,
    target = profile.target_version or V_EQUAL,
    missing = nil,
  }
  local currents = {
    { name = "source", value = { state = profile.sources[1], version = current_versions.source } },
    { name = "target", value = { state = profile.target, version = current_versions.target } },
    { name = "missing", value = { state = nil, version = current_versions.missing } },
  }
  local versions = {
    { name = "older", value = V_OLDER },
    { name = "equal", value = V_EQUAL },
    { name = "newer", value = V_NEWER },
  }
  for _, current_case in ipairs(currents) do
    for _, version_case in ipairs(versions) do
      local expected, incoming_for_base = profile_production_result(
        profile,
        current_case.value,
        version_case.value
      )
      local extras = {
        overlay_version = profile.overlay_version or version_case.value,
        target_version = profile.target_version,
      }
      if profile.id == "cas.legacy_consensus_result_v1" then
        extras.effects_complete = true
      end
      local actual = catalog.resolve(profile.id, evidence(
        current_case.value,
        profile.variant,
        incoming_for_base,
        extras
      ))
      assert_result(actual, expected, table.concat({
        profile.id,
        current_case.name,
        version_case.name,
      }, "/"))
    end
  end
end

return {
  test_catalog_is_closed_total_and_records_the_assumption = function()
    local expected = {}
    for _, id in ipairs(base_policy_ids) do expected[id] = true end
    for _, id in ipairs(complete_policy_ids) do expected[id] = true end

    local ids = catalog.policy_ids()
    t.eq(#ids, #base_policy_ids + #complete_policy_ids)
    for _, id in ipairs(ids) do
      t.eq(expected[id], true)
      expected[id] = nil
      local definition = catalog.definition(id)
      t.eq(definition.id, id)
      t.eq(type(definition.evidence_type), "string")
      t.eq(type(definition.production), "table")
    end
    t.eq(next(expected), nil)
    t.eq(catalog.assumptions.versions_equivalent, "ASSUMED-UNVERIFIED")

    assert_result(
      catalog.resolve("cas.unknown", {}),
      result("illegal", "unknown-cas-policy", "illegal(unknown-cas-policy)")
    )
    assert_result(
      catalog.resolve("cas.legacy_fix_v1", { current = "not-evidence" }),
      result("illegal", "invalid-evidence", "illegal(invalid-evidence)")
    )
  end,

  test_plain_versioned_and_cyclic_bases_match_production_matrices = function()
    assert_base_matrix(
      "cas.base_plain_legacy_v1",
      function(current, sources, target)
        return production_result(current, devloop_state.transition_status(current, sources, target), nil)
      end,
      { "thinking" },
      "ready"
    )
    assert_base_matrix(
      "cas.base_versioned_legacy_v1",
      function(current, sources, target, incoming)
        return production_versioned(current, sources, target, incoming)
      end,
      { "thinking" },
      "ready"
    )
    assert_base_matrix(
      "cas.base_cyclic_legacy_v1",
      function(current, sources, target, incoming, target_version)
        return production_cyclic(current, sources, target, incoming, target_version)
      end,
      { "reviewing" },
      "fixing",
      V_EQUAL .. "/fix/1"
    )

    local newer_versioned = catalog.resolve("cas.base_versioned_legacy_v1", {
      current = { state = "thinking", version = V_EQUAL },
      source_states = { "thinking" },
      target_state = "ready",
      incoming_version = V_NEWER,
    })
    t.eq(newer_versioned.status, "apply")

    local raw_version = "/reviewing#head//fix/1/"
    local equivalent_target = transition_version.safe_version_segment(raw_version)
    assert_result(
      catalog.resolve("cas.base_cyclic_legacy_v1", {
        current = { state = "reviewing", version = raw_version },
        source_states = { "fixing" },
        target_state = "reviewing",
        incoming_version = V_OLDER,
        target_version = equivalent_target,
      }),
      production_cyclic(
        { state = "reviewing", version = raw_version },
        { "fixing" },
        "reviewing",
        V_OLDER,
        equivalent_target
      ),
      "cyclic/assumed-versions-equivalent-observed-form"
    )
  end,

  test_plain_and_versioned_complete_profiles_match_production = function()
    local loop_current = { state = "thinking", version = V_EQUAL }
    assert_result(
      catalog.resolve("cas.legacy_loop_plain_v1", evidence(loop_current, "thinking_to_blocked")),
      production_result(loop_current, devloop_state.transition_status(loop_current, { "thinking" }, "blocked"), nil)
    )

    for _, variant in ipairs({ "thinking_to_ready", "thinking_to_dependency_wait" }) do
      local target = variant == "thinking_to_ready" and "ready" or "dependency_wait"
      for _, current in ipairs({
        { state = "thinking", version = V_EQUAL },
        { state = target, version = V_EQUAL },
        { state = nil, version = nil },
        { state = "merged", version = V_EQUAL },
      }) do
        local expected = production_versioned(current, { "thinking" }, target, V_EQUAL)
        assert_result(
          catalog.resolve("cas.legacy_consensus_result_v1", evidence(current, variant, V_EQUAL, {
            effects_complete = true,
          })),
          expected,
          "consensus/" .. variant .. "/" .. tostring(current.state)
        )
      end
    end

    local incomplete = catalog.resolve("cas.legacy_consensus_result_v1", evidence({
      state = "ready",
      version = V_EQUAL,
    }, "thinking_to_ready", V_EQUAL, { effects_complete = false }))
    assert_result(incomplete, result("apply", "effects-incomplete", "applied(result effects incomplete)"))

    for _, profile in ipairs({
      { id = "cas.legacy_issue_reconcile_v1", variant = "thinking_to_blocked", source = { "thinking" }, target = "blocked" },
      { id = "cas.legacy_timeout_reconcile_v1", variant = "reviewing_to_blocked", source = { "reviewing" }, target = "blocked" },
      { id = "cas.legacy_observe_issue_entry_v1", variant = "unmanaged_to_thinking", source = { "unmanaged" }, target = "thinking" },
      { id = "cas.legacy_awaiting_pr_v1", variant = "awaiting_pr_to_merged", source = { "awaiting-pr" }, target = "merged" },
    }) do
      local current = { state = profile.source[1], version = V_EQUAL }
      assert_result(
        catalog.resolve(profile.id, evidence(current, profile.variant, V_EQUAL)),
        production_versioned(current, profile.source, profile.target, V_EQUAL),
        profile.id .. "/" .. profile.variant
      )
    end
  end,

  test_raw_and_safe_overlay_profiles_match_production_order = function()
    local observe_source = { state = "pr-open", version = V_EQUAL }
    local observe_expected = raw_overlay(
      production_versioned(observe_source, { "pr-open", "unmanaged" }, "reviewing", V_NEWER),
      observe_source,
      { "pr-open" },
      V_NEWER,
      { apply = true }
    )
    assert_result(
      catalog.resolve("cas.legacy_observe_pr_v1", evidence(observe_source, "pr_open_to_reviewing", V_NEWER)),
      observe_expected
    )

    local review_source = { state = "reviewing", version = V_EQUAL }
    local safe_current = {
      state = review_source.state,
      version = transition_version.safe_version_segment(review_source.version),
    }
    local safe_incoming = transition_version.safe_version_segment(V_EQUAL)
    local review_expected = safe_overlay(
      production_cyclic(safe_current, { "reviewing" }, "fixing", safe_incoming),
      review_source,
      { "reviewing" },
      V_EQUAL,
      { apply = true }
    )
    assert_result(
      catalog.resolve("cas.legacy_review_result_v1", evidence(review_source, "reviewing_to_fixing", safe_incoming, {
        overlay_version = V_EQUAL,
      })),
      review_expected
    )

    for _, profile in ipairs({
      { id = "cas.legacy_fix_v1", variant = "fixing_to_reviewing", source = "fixing", target = "reviewing", target_version = V_EQUAL .. "/fix/3" },
      { id = "cas.legacy_review_meta_v1", variant = "review_meta_to_fixing", source = "review-meta", target = "fixing" },
      { id = "cas.legacy_merge_v1", variant = "merge_ready_to_merging", source = "merge-ready", target = "merging" },
    }) do
      local current = { state = profile.source, version = V_EQUAL }
      local expected = production_cyclic(current, { profile.source }, profile.target, V_EQUAL, profile.target_version)
      expected = raw_overlay(expected, current, { profile.source }, V_EQUAL, { apply = true })
      assert_result(
        catalog.resolve(profile.id, evidence(current, profile.variant, V_EQUAL, {
          target_version = profile.target_version,
        })),
        expected
      )
    end

    local reconcile_current = { state = "fixing", version = V_EQUAL }
    local reconcile_terminal_version = V_EQUAL .. "/timeout/fixing/1"
    local reconcile_expected = safe_overlay(
      production_versioned(reconcile_current, { "reviewing", "fixing", "merge-ready", "merging" }, "blocked", reconcile_terminal_version),
      reconcile_current,
      { "reviewing", "fixing", "merge-ready", "merging" },
      V_EQUAL,
      { apply = true }
    )
    assert_result(
      catalog.resolve("cas.legacy_pr_fix_reconcile_v1", evidence(
        reconcile_current,
        "review_reject_to_blocked",
        reconcile_terminal_version,
        { overlay_version = V_EQUAL }
      )),
      reconcile_expected
    )

    for _, mismatch in ipairs({
      {
        id = "cas.legacy_review_result_v1",
        variant = "reviewing_to_fixing",
        incoming_version = transition_version.safe_version_segment(V_EQUAL),
        overlay_version = V_OTHER,
      },
      { id = "cas.legacy_fix_v1", variant = "fixing_to_reviewing", incoming_version = V_EQUAL, overlay_version = V_OTHER },
      { id = "cas.legacy_review_meta_v1", variant = "review_meta_to_fixing", incoming_version = V_EQUAL, overlay_version = V_OTHER },
      { id = "cas.legacy_merge_v1", variant = "merge_ready_to_merging", incoming_version = V_EQUAL, overlay_version = V_OTHER },
      {
        id = "cas.legacy_pr_fix_reconcile_v1",
        variant = "review_reject_to_blocked",
        incoming_version = V_NEWER,
        overlay_version = V_OTHER,
      },
    }) do
      local definition = catalog.definition(mismatch.id)
      local variant_definition = definition.variants[mismatch.variant]
      local current = { state = variant_definition.source_states[1], version = V_EQUAL }
      assert_result(
        catalog.resolve(mismatch.id, evidence(current, mismatch.variant, mismatch.incoming_version, {
          overlay_version = mismatch.overlay_version,
        })),
        result("stale", "version-mismatch", "skip-stale(version-mismatch)"),
        mismatch.id .. "/overlay-mismatch"
      )
    end
  end,

  test_every_complete_base_profile_freezes_order_state_and_overlay_matrices = function()
    local profiles = {
      { id = "cas.legacy_loop_plain_v1", variant = "thinking_to_blocked", base = "plain", sources = { "thinking" }, target = "blocked" },
      { id = "cas.legacy_consensus_result_v1", variant = "thinking_to_ready", base = "versioned", sources = { "thinking" }, target = "ready" },
      { id = "cas.legacy_issue_reconcile_v1", variant = "thinking_to_blocked", base = "versioned", sources = { "thinking" }, target = "blocked" },
      { id = "cas.legacy_timeout_reconcile_v1", variant = "reviewing_to_blocked", base = "versioned", sources = { "reviewing" }, target = "blocked" },
      { id = "cas.legacy_observe_issue_entry_v1", variant = "unmanaged_to_thinking", base = "versioned", sources = { "unmanaged" }, target = "thinking" },
      { id = "cas.legacy_awaiting_pr_v1", variant = "awaiting_pr_to_merged", base = "versioned", sources = { "awaiting-pr" }, target = "merged" },
      {
        id = "cas.legacy_observe_pr_v1",
        variant = "pr_open_to_reviewing",
        base = "versioned",
        sources = { "pr-open", "unmanaged" },
        target = "reviewing",
        overlay = "raw",
        overlay_sources = { "pr-open" },
        overlay_only_states = { "pr-open" },
        overlay_statuses = { apply = true },
      },
      {
        id = "cas.legacy_review_result_v1",
        variant = "reviewing_to_fixing",
        base = "cyclic",
        base_current_version_form = "safe",
        sources = { "reviewing" },
        target = "fixing",
        overlay = "safe",
        overlay_statuses = { apply = true },
      },
      {
        id = "cas.legacy_fix_v1",
        variant = "fixing_to_reviewing",
        base = "cyclic",
        sources = { "fixing" },
        target = "reviewing",
        target_version = V_EQUAL .. "/fix/3",
        overlay = "raw",
        overlay_statuses = { apply = true },
      },
      {
        id = "cas.legacy_review_meta_v1",
        variant = "review_meta_to_fixing",
        base = "cyclic",
        sources = { "review-meta" },
        target = "fixing",
        overlay = "raw",
        overlay_statuses = { apply = true },
      },
      {
        id = "cas.legacy_merge_v1",
        variant = "merge_ready_or_merging_to_merging",
        base = "cyclic",
        sources = { "merge-ready", "merging" },
        target = "merging",
        overlay = "raw",
        overlay_statuses = { apply = true, idempotent = true },
      },
      {
        id = "cas.legacy_pr_fix_reconcile_v1",
        variant = "review_reject_to_blocked",
        base = "versioned",
        sources = { "reviewing", "fixing", "merge-ready", "merging" },
        target = "blocked",
        overlay = "safe",
        overlay_version = V_EQUAL,
        overlay_statuses = { apply = true },
      },
    }
    for _, profile in ipairs(profiles) do
      assert_complete_profile_matrix(profile)
    end
  end,

  test_review_loop_and_review_activation_preserve_non_ordering_and_distinct_mismatch = function()
    local safe = transition_version.safe_version_segment(V_EQUAL)
    assert_result(
      catalog.resolve("cas.legacy_review_loop_safe_v1", {
        current = { state = "reviewing", version = V_EQUAL },
        review_version = safe,
      }),
      result("apply", "apply", "applied")
    )
    assert_result(
      catalog.resolve("cas.legacy_review_loop_safe_v1", {
        current = { state = "reviewing", version = V_NEWER },
        review_version = safe,
      }),
      result("stale", "reviewing-version", "skip-stale(reviewing-version)")
    )
    assert_result(
      catalog.resolve("cas.legacy_review_loop_safe_v1", {
        current = { state = nil, version = nil },
        review_version = safe,
      }),
      result("pending", "source-marker-not-visible", "retry-pending(from-state marker not yet visible)")
    )

    local mismatch_evidence = {
      current = { state = "reviewing", version = V_EQUAL },
      reviewing_version = V_OTHER,
      handoff = { status = "invalid" },
    }
    assert_result(
      catalog.resolve("cas.legacy_review_activation_handoff_v1", mismatch_evidence),
      result("stale", "version-mismatch", "skip-stale(version-mismatch)")
    )
    mismatch_evidence.handoff.status = "valid"
    assert_result(
      catalog.resolve("cas.legacy_review_activation_handoff_v1", mismatch_evidence),
      result("apply", "verified-own-reviewing-hand-off", "apply(verified-own-reviewing-hand-off)")
    )
    assert_result(
      catalog.resolve("cas.legacy_review_activation_handoff_v1", {
        current = { state = nil, version = nil },
        reviewing_version = V_EQUAL,
        handoff = { status = "invalid" },
      }),
      result("pending", "reviewing-marker-not-visible", "retry-pending(reviewing marker not yet visible)")
    )
    assert_result(
      catalog.resolve("cas.legacy_review_activation_handoff_v1", {
        current = { state = nil, version = nil },
        reviewing_version = V_EQUAL,
      }),
      result("pending", "reviewing-marker-not-visible", "retry-pending(reviewing marker not yet visible)")
    )
  end,

  test_implement_handoff_is_initial_only_and_rechecks_are_structural = function()
    local missing = { state = nil, version = nil }
    assert_result(
      catalog.resolve("cas.legacy_implement_activation_handoff_v1", {
        current = missing,
        variant = "ready_to_implementing",
        incoming_version = V_EQUAL,
        phase = "initial",
        handoff = { status = "valid" },
        retry = false,
      }),
      result("apply", "verified-own-ready-hand-off", "apply(verified-own-ready-hand-off)"),
      "implement/initial-handoff"
    )
    assert_result(
      catalog.resolve("cas.legacy_implement_activation_handoff_v1", {
        current = missing,
        variant = "ready_to_implementing",
        incoming_version = V_EQUAL,
        phase = "initial",
        handoff = { status = "valid" },
        retry = true,
      }),
      result("pending", "source-marker-not-visible", "retry-pending(from-state marker not yet visible)"),
      "implement/retry-does-not-borrow-handoff"
    )
    assert_result(
      catalog.resolve("cas.legacy_implement_activation_handoff_v1", {
        current = missing,
        variant = "ready_to_implementing",
        incoming_version = V_EQUAL,
        phase = "recheck",
        handoff = { status = "valid" },
        accepted_handoff = true,
        retry = false,
      }),
      result("apply", "own-ready-hand-off", "apply(own-ready-hand-off)"),
      "implement/recheck-accepted-handoff"
    )
    assert_result(
      catalog.resolve("cas.legacy_implement_activation_handoff_v1", {
        current = missing,
        variant = "ready_to_implementing",
        incoming_version = V_EQUAL,
        phase = "initial",
        retry = false,
      }),
      result("pending", "source-marker-not-visible", "retry-pending(from-state marker not yet visible)"),
      "implement/absent-handoff"
    )

    for _, current_case in ipairs({
      { name = "source", value = { state = "ready", version = V_EQUAL } },
      { name = "target", value = { state = "implementing", version = V_EQUAL } },
      { name = "missing", value = { state = nil, version = nil } },
    }) do
      for _, version_case in ipairs({
        { name = "older", value = V_OLDER },
        { name = "equal", value = V_EQUAL },
        { name = "newer", value = V_NEWER },
      }) do
        local expected
        if current_case.value.state == "implementing" then
          expected = result("idempotent", "already-at-target", "skip-idempotent(already at to_state)")
        else
          expected = production_versioned(current_case.value, { "ready" }, "implementing", version_case.value)
        end
        assert_result(
          catalog.resolve("cas.legacy_implement_activation_handoff_v1", {
            current = current_case.value,
            variant = "ready_to_implementing",
            incoming_version = version_case.value,
            phase = "initial",
            handoff = { status = "invalid" },
            retry = false,
          }),
          expected,
          "implement/matrix/" .. current_case.name .. "/" .. version_case.name
        )
      end
    end
  end,

  test_catalog_binds_core_extracted_edges_and_activation_transitions = function()
    local seen = binding_set()
    for _, id in ipairs({
      "github-devloop/thinking/autonomous/consensus-reached",
      "github-devloop/thinking/autonomous/consensus-stalled",
      "github-devloop/ready/entry/implementation_kicked_off",
      "github-devloop/impl-failed/entry/retry-implementation",
      "github-devloop-pr/reviewing/autonomous/approved",
      "github-devloop-pr/reviewing/autonomous/changes_requested",
      "github-devloop-pr/fixing/autonomous/revision_published",
      "github-devloop-pr/review-meta/autonomous/fix",
      "github-devloop-pr/merge-ready/entry/handoff_to_merge_gate",
      "transition/github-devloop-pr/review_pr/reviewing-activation",
      "transition/github-devloop-pr/review_loop/reviewing-convergence",
    }) do
      t.eq(seen[id], true)
    end
    t.eq(#catalog.deferred_bindings(), 0)
  end,
}
