local marker = require("core.marker")
local t = fkst.test

local origin = "github-devloop/issue/owner/repo/42"
local workflow_id = "workflow-one"
local digest = "d-1234567890"
local slot = "slot-one"
local predecessor_ref_digest = "d-1111111111"
local gen_contract_digest = "d-2222222222"
local gen_spec_digest = "d-3333333333"
local child_dedup = "workflow/owner/repo/42/slot-one"

local function build_blueprint_or_error(origin_proposal_id, workflow, plan_digest)
  local built, err = marker.build_blueprint_marker(origin_proposal_id, workflow, plan_digest)
  if built == nil then
    error(err and err.code or "failed to build marker")
  end
  return built
end

local function build_rejects(origin_proposal_id, workflow, plan_digest, expected)
  local built, err = marker.build_blueprint_marker(origin_proposal_id, workflow, plan_digest)
  t.is_nil(built)
  t.is_true(type(err) == "table")
  if expected ~= nil then
    t.eq(err.path, expected.path)
    t.eq(err.code, expected.code)
  end
end

local function build_materialization_or_error(state, child_issue)
  local built, err = marker.build_materialization_marker(
    origin,
    digest,
    slot,
    predecessor_ref_digest,
    gen_contract_digest,
    gen_spec_digest,
    child_dedup,
    child_issue,
    state
  )
  if built == nil then
    error(err and err.code or "failed to build materialization marker")
  end
  return built
end

local function materialization_rejects(args, expected)
  local built, err = marker.build_materialization_marker(
    args.origin or origin,
    args.blueprint_digest or digest,
    args.slot or slot,
    args.predecessor_ref_digest or predecessor_ref_digest,
    args.gen_contract_digest or gen_contract_digest,
    args.gen_spec_digest or gen_spec_digest,
    args.child_dedup or child_dedup,
    args.child_issue,
    args.state or "pending"
  )
  t.is_nil(built)
  t.is_true(type(err) == "table")
  t.eq(err.path, expected.path)
  t.eq(err.code, expected.code)
end

local function terminal_rejects(origin_proposal_id, terminal_state, reason_code, expected)
  local built, err = marker.build_terminal_marker(origin_proposal_id, terminal_state, reason_code)
  t.is_nil(built)
  t.is_true(type(err) == "table")
  t.eq(err.path, expected.path)
  t.eq(err.code, expected.code)
end

local function lineage_rejects(origin_proposal_id, blueprint_digest, slot_id, expected)
  local built, err = marker.build_lineage_header(origin_proposal_id, blueprint_digest, slot_id)
  t.is_nil(built)
  t.is_true(type(err) == "table")
  t.eq(err.path, expected.path)
  t.eq(err.code, expected.code)
end

local tests = {
  test_build_parse_round_trip = function()
    local built = build_blueprint_or_error(origin, workflow_id, digest)
    t.eq(
      built,
      '<!-- fkst:github-devloop-workflow:blueprint:v1 origin="github-devloop/issue/owner/repo/42" workflow="workflow-one" digest="d-1234567890" -->'
    )

    local parsed = marker.parse_blueprint_marker("body\n" .. built .. "\n", origin)
    t.eq(parsed.origin, origin)
    t.eq(parsed.workflow, workflow_id)
    t.eq(parsed.digest, digest)
  end,

  test_rejects_malformed_origin_field = function()
    build_rejects("bad origin", workflow_id, digest, {
      path = "origin_proposal_id",
      code = "invalid_key",
    })
  end,

  test_rejects_malformed_workflow_field = function()
    build_rejects(origin, "bad workflow", digest, {
      path = "workflow_id",
      code = "invalid_key",
    })
  end,

  test_rejects_malformed_digest_field = function()
    build_rejects(origin, workflow_id, 'bad"digest', {
      path = "plan_digest",
      code = "invalid_marker_attr",
    })
  end,

  test_rejects_oversized_field = function()
    build_rejects(origin, string.rep("w", marker.MAX_WORKFLOW_ID_BYTES + 1), digest, {
      path = "workflow_id",
      code = "too_large",
    })
  end,

  test_parse_returns_nil_when_absent = function()
    t.is_nil(marker.parse_blueprint_marker("ordinary comment", origin))
  end,

  test_parse_returns_nil_for_foreign_namespace = function()
    local body = '<!-- fkst:github-devloop:blueprint:v1 origin="' .. origin
      .. '" workflow="' .. workflow_id
      .. '" digest="' .. digest
      .. '" -->'
    t.is_nil(marker.parse_blueprint_marker(body, origin))
  end,

  test_parse_returns_nil_for_malformed_marker = function()
    local body = '<!-- fkst:github-devloop-workflow:blueprint:v1 origin="' .. origin
      .. '" workflow="bad workflow" digest="' .. digest
      .. '" -->'
    t.is_nil(marker.parse_blueprint_marker(body, origin))
  end,

  test_parse_picks_right_origin_among_multiple_markers = function()
    local other = build_blueprint_or_error("github-devloop/issue/owner/repo/7", "other-workflow", "d-0000000007")
    local first = build_blueprint_or_error(origin, "older-workflow", "d-1111111111")
    local latest = build_blueprint_or_error(origin, "newer-workflow", "d-2222222222")
    local parsed = marker.parse_blueprint_marker(other .. "\n" .. first .. "\n" .. latest, origin)
    t.eq(parsed.origin, origin)
    t.eq(parsed.workflow, "newer-workflow")
    t.eq(parsed.digest, "d-2222222222")
  end,

  test_materialization_marker_round_trips_each_state = function()
    for _, state in ipairs({ "pending", "generated", "created" }) do
      local built = build_materialization_or_error(state, "108")
      local parsed = marker.parse_materialization_marker("prefix\n" .. built, origin, slot)
      t.eq(parsed.origin, origin)
      t.eq(parsed.blueprint_digest, digest)
      t.eq(parsed.slot, slot)
      t.eq(parsed.predecessor_ref_digest, predecessor_ref_digest)
      t.eq(parsed.gen_contract_digest, gen_contract_digest)
      t.eq(parsed.gen_spec_digest, gen_spec_digest)
      t.eq(parsed.child_dedup, child_dedup)
      t.eq(parsed.child_issue, "108")
      t.eq(parsed.state, state)
    end
  end,

  test_materialization_marker_allows_empty_child_issue_until_created = function()
    local built = build_materialization_or_error("generated", nil)
    t.is_true(built:find('child_issue=""', 1, true) ~= nil)
    local parsed = marker.parse_materialization_marker(built, origin, slot)
    t.is_nil(parsed.child_issue)
  end,

  test_materialization_marker_rejects_bad_fields = function()
    materialization_rejects({ state = "started" }, {
      path = "state",
      code = "invalid_materialization_state",
    })
    materialization_rejects({ predecessor_ref_digest = string.rep("d", marker.MAX_MATERIALIZATION_DIGEST_BYTES + 1) }, {
      path = "predecessor_ref_digest",
      code = "too_large",
    })
    materialization_rejects({ blueprint_digest = string.rep("d", marker.MAX_MATERIALIZATION_DIGEST_BYTES + 1) }, {
      path = "blueprint_digest",
      code = "too_large",
    })
    materialization_rejects({ gen_contract_digest = 'bad"digest' }, {
      path = "generator_contract_digest",
      code = "invalid_marker_attr",
    })
    materialization_rejects({ gen_spec_digest = "bad digest" }, {
      path = "generated_spec_digest",
      code = "invalid_key",
    })
    materialization_rejects({ child_dedup = "bad dedup" }, {
      path = "child_dedup_key",
      code = "invalid_key",
    })
    materialization_rejects({ child_issue = "12x" }, {
      path = "child_issue",
      code = "invalid_issue_number",
    })
    materialization_rejects({ slot = string.rep("s", marker.MAX_SLOT_ID_BYTES + 1) }, {
      path = "slot_id",
      code = "too_large",
    })
  end,

  test_parse_materialization_marker_fail_closed_for_malformed = function()
    local body = '<!-- fkst:github-devloop-workflow:materialization:v1 origin="' .. origin
      .. '" blueprint_digest="' .. digest
      .. '" slot="' .. slot
      .. '" predecessor_ref_digest="' .. predecessor_ref_digest
      .. '" gen_contract_digest="' .. gen_contract_digest
      .. '" gen_spec_digest="bad digest" child_dedup="' .. child_dedup
      .. '" child_issue="" state="pending" -->'
    t.is_nil(marker.parse_materialization_marker(body, origin, slot))
    t.is_nil(marker.parse_materialization_marker(build_materialization_or_error("pending", nil), origin, "other-slot"))
  end,

  test_latest_materialization_by_slot_uses_highest_state_then_latest_order = function()
    local older_created = build_materialization_or_error("created", "108")
    local later_pending = marker.build_materialization_marker(
      origin,
      digest,
      slot,
      "d-4444444444",
      gen_contract_digest,
      gen_spec_digest,
      child_dedup,
      "",
      "pending"
    )
    local other_slot = marker.build_materialization_marker(
      origin,
      digest,
      "slot-two",
      predecessor_ref_digest,
      gen_contract_digest,
      gen_spec_digest,
      "workflow/owner/repo/42/slot-two",
      "",
      "generated"
    )
    local facts = marker.parse_materialization_markers(older_created .. "\n" .. later_pending .. "\n" .. other_slot, origin)
    local by_slot = marker.latest_materialization_by_slot(facts)
    t.eq(by_slot[slot].state, "created")
    t.eq(by_slot[slot].child_issue, "108")
    t.eq(by_slot["slot-two"].state, "generated")

    local first_generated = build_materialization_or_error("generated", nil)
    local later_generated = marker.build_materialization_marker(
      origin,
      digest,
      slot,
      predecessor_ref_digest,
      gen_contract_digest,
      "d-5555555555",
      child_dedup,
      "",
      "generated"
    )
    local same_rank = marker.latest_materialization_by_slot(marker.parse_materialization_markers(first_generated .. "\n" .. later_generated, origin))
    t.eq(same_rank[slot].gen_spec_digest, "d-5555555555")
  end,

  test_terminal_marker_round_trip_uses_reason_code_attribute = function()
    local built, err = marker.build_terminal_marker(origin, "blocked", "child-fatal")
    t.is_nil(err)
    t.eq(
      built,
      '<!-- fkst:github-devloop-workflow:terminal:v1 origin="github-devloop/issue/owner/repo/42" state="blocked" reason_code="child-fatal" -->'
    )
    local parsed = marker.parse_terminal_marker("Full prose lives outside the marker.\n" .. built, origin)
    t.eq(parsed.origin, origin)
    t.eq(parsed.state, "blocked")
    t.eq(parsed.reason_code, "child-fatal")
  end,

  test_terminal_marker_rejects_bad_fields = function()
    terminal_rejects(origin, "waiting", "child-fatal", {
      path = "terminal_state",
      code = "invalid_terminal_state",
    })
    terminal_rejects(origin, "error", "bad reason", {
      path = "reason_code",
      code = "invalid_key",
    })
    terminal_rejects(origin, "done", string.rep("r", marker.MAX_TERMINAL_REASON_CODE_BYTES + 1), {
      path = "reason_code",
      code = "too_large",
    })
  end,

  test_parse_terminal_marker_fail_closed_for_malformed = function()
    local body = '<!-- fkst:github-devloop-workflow:terminal:v1 origin="' .. origin
      .. '" state="done" reason_code="bad reason" -->'
    t.is_nil(marker.parse_terminal_marker(body, origin))
  end,

  test_lineage_header_round_trip_and_parse_from_body = function()
    local built, err = marker.build_lineage_header(origin, digest, slot)
    t.is_nil(err)
    t.eq(
      built,
      '<!-- fkst:github-devloop-workflow:lineage:v1 origin="github-devloop/issue/owner/repo/42" blueprint_digest="d-1234567890" slot="slot-one" -->'
    )
    local parsed = marker.parse_lineage_header("issue body\n" .. built .. "\n\nwork text")
    t.eq(parsed.origin, origin)
    t.eq(parsed.blueprint_digest, digest)
    t.eq(parsed.slot, slot)
  end,

  test_lineage_header_rejects_and_parse_fails_closed = function()
    lineage_rejects(origin, 'bad"digest', slot, {
      path = "blueprint_digest",
      code = "invalid_marker_attr",
    })
    lineage_rejects(origin, digest, "bad slot", {
      path = "slot_id",
      code = "invalid_key",
    })
    local malformed = '<!-- fkst:github-devloop-workflow:lineage:v1 origin="' .. origin
      .. '" blueprint_digest="bad digest" slot="' .. slot
      .. '" -->'
    t.is_nil(marker.parse_lineage_header(malformed))
    t.is_nil(marker.parse_lineage_header("ordinary body"))
  end,
}

return tests
