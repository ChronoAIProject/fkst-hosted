local materialization = require("core.materialization")
local marker = require("core.marker")
local t = fkst.test

local origin = "github-devloop/issue/owner/repo/42"
local blueprint_digest = "d-1234567890"
local slot = {
  id = "first",
  title = "First",
  content = {
    kind = "static",
    intent = "Do it.",
  },
}
local predecessor_ref_digest = "d-2222222222"
local spec = {
  title = "Generated issue",
  body = "Do generated work.",
}

local function key()
  return materialization.materialization_key(origin, blueprint_digest, slot.id, predecessor_ref_digest)
end

local function fact(state, gen_digest)
  return {
    origin = origin,
    blueprint_digest = blueprint_digest,
    slot = slot.id,
    predecessor_ref_digest = predecessor_ref_digest,
    gen_spec_digest = gen_digest or materialization.generated_spec_digest(spec),
    child_dedup = materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest),
    child_issue = "108",
    state = state,
  }
end

local tests = {
  test_materialization_key_is_deterministic = function()
    t.eq(
      materialization.materialization_key(origin, blueprint_digest, slot.id, predecessor_ref_digest),
      materialization.materialization_key(origin, blueprint_digest, slot.id, predecessor_ref_digest)
    )
    t.is_true(materialization.materialization_key(origin, blueprint_digest, slot.id, predecessor_ref_digest)
      ~= materialization.materialization_key(origin, blueprint_digest, "second", predecessor_ref_digest))
  end,

  test_child_dedup_key_is_deterministic_from_origin_slot_and_predecessor = function()
    local first = materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest)
    local second = materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest)
    t.eq(first, second)
    t.is_true(first:find("github-devloop/issue/owner/repo/42", 1, true) ~= nil)
    t.is_true(first:find("first", 1, true) ~= nil)
    t.is_true(first:find(predecessor_ref_digest, 1, true) ~= nil)
  end,

  test_latch_noops_when_created_already_exists = function()
    local decision = materialization.latch_generated({ fact("created") }, key(), spec)
    t.eq(decision.action, "noop")
    t.eq(decision.reason_code, "already-created")
    t.eq(decision.child_issue, "108")
  end,

  test_latch_replays_create_when_same_generated_digest_exists = function()
    local decision = materialization.latch_generated({ fact("generated") }, key(), spec)
    t.eq(decision.action, "proceed_create")
    t.eq(decision.generated_spec_digest, materialization.generated_spec_digest(spec))
    t.eq(decision.child_dedup_key, materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest))
  end,

  test_latch_rejects_different_generated_digest_for_same_key = function()
    local decision = materialization.latch_generated({ fact("generated", "d-9999999999") }, key(), spec)
    t.eq(decision.action, "error")
    t.eq(decision.reason_code, "generated-spec-digest-conflict")
  end,

  test_latch_rejects_any_conflicting_same_key_fact = function()
    local decision = materialization.latch_generated({
      fact("generated"),
      fact("generated", "d-9999999999"),
    }, key(), spec)
    t.eq(decision.action, "error")
    t.eq(decision.reason_code, "generated-spec-digest-conflict")
  end,

  test_latch_writes_generated_when_absent = function()
    local decision = materialization.latch_generated({}, key(), spec)
    t.eq(decision.action, "proceed_create")
    t.eq(decision.generated_spec_digest, materialization.generated_spec_digest(spec))
  end,

  test_write_generated_entry_derives_marker_fields = function()
    local entry = materialization.write_generated_entry(origin, blueprint_digest, slot, predecessor_ref_digest, spec)
    t.eq(entry.origin, origin)
    t.eq(entry.blueprint_digest, blueprint_digest)
    t.eq(entry.slot, slot.id)
    t.eq(entry.predecessor_ref_digest, predecessor_ref_digest)
    t.eq(entry.gen_contract_digest, materialization.generator_contract_digest(slot))
    t.eq(entry.gen_spec_digest, materialization.generated_spec_digest(spec))
    t.eq(entry.state, "generated")
  end,

  test_marker_roundtrip_fact_key_is_canonical_cas_key = function()
    local entry = materialization.write_generated_entry(origin, blueprint_digest, slot, predecessor_ref_digest, spec)
    local built, err = marker.build_materialization_marker(
      entry.origin,
      entry.blueprint_digest,
      entry.slot,
      entry.predecessor_ref_digest,
      entry.gen_contract_digest,
      entry.gen_spec_digest,
      entry.child_dedup,
      nil,
      entry.state
    )
    t.is_nil(err)

    local parsed = marker.parse_materialization_marker(built, origin, slot.id)
    local canonical = materialization.materialization_key(origin, blueprint_digest, slot.id, predecessor_ref_digest)
    t.eq(materialization.fact_key(parsed), canonical)

    local decision = materialization.latch_generated({ parsed }, canonical, spec)
    t.eq(decision.action, "proceed_create")
    t.eq(decision.generated_spec_digest, materialization.generated_spec_digest(spec))
    t.eq(decision.child_dedup_key, materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest))
  end,
}

return tests
