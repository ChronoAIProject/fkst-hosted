local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local operator_commands = require("devloop.operator_commands")
local operator_reentry_inventory = require("core.restart.operator_reentry_inventory")
local restart_edges = require("devloop.restart_edges")
local transition_version = require("contract.transition_version")

local core = h.core
local t = h.t

local owner = "github-devloop-pr"
local proposal_id = "github-devloop/issue/owner/repo/42"
local pr_number = 7
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"
local base_branch = "dev"
local head_sha = "feedface"
local structural_fields = {
  "id",
  "owner",
  "row_id",
  "kind",
  "source",
  "target",
  "cause_evidence",
  "provenance",
}
local pending_order_goldens = {
  ["github-devloop-pr/reviewing/operator_reentry/rereview_blocked"] = { participates = false },
  ["github-devloop-pr/reviewing/operator_reentry/rereview_review_meta"] = { participates = false },
  ["github-devloop-pr/reviewing/operator_reentry/rereview_reviewing"] = { participates = false },
}

local function key_set(keys)
  local out = {}
  for _, key in ipairs(keys) do
    out[key] = true
  end
  return out
end

local function assert_exact_keys(value, expected)
  local count = 0
  for key in pairs(value) do
    count = count + 1
    t.eq(expected[key], true)
  end
  local expected_count = 0
  for _ in pairs(expected) do
    expected_count = expected_count + 1
  end
  t.eq(count, expected_count)
end

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function assert_same_value(actual, expected)
  if type(expected) ~= "table" then
    t.eq(actual, expected)
    return
  end
  t.eq(type(actual), "table")
  local actual_count = 0
  for _ in pairs(actual) do
    actual_count = actual_count + 1
  end
  local expected_count = 0
  for key, nested in pairs(expected) do
    expected_count = expected_count + 1
    assert_same_value(actual[key], nested)
  end
  t.eq(actual_count, expected_count)
end

local function trusted_comment(body, id, created_at)
  return {
    id = id,
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-04T03:01:00Z",
  }
end

local function trusted_command(id)
  return trusted_comment("fkst: rereview", id, "2026-06-04T03:00:00Z")
end

local function append_comment(comments, comment)
  local out = {}
  for _, current in ipairs(comments or {}) do
    table.insert(out, current)
  end
  table.insert(out, comment)
  return out
end

local function pr_event(updated_at)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = pr_number,
    state = "OPEN",
    updated_at = updated_at or "2026-06-04T03:00:00Z",
    dedup_key = "owner/repo#pr#7@" .. tostring(updated_at or "2026-06-04T03:00:00Z"),
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
  }
end

local function assert_department_ok(result, site)
  if result.exit_code ~= 0 then
    error("restart operator reentry conformance: " .. site .. " failed: "
      .. tostring(result.error or result.stderr or "unknown department failure"))
  end
end

local function find_accepted_response(raises)
  return h.find_raise(raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find("operator command accepted: rereview", 1, true) ~= nil
  end)
end

local function applied_cause_evidence(comments, response_body)
  local command = operator_commands.operator_command_fact(comments, "rereview")
  t.is_true(command ~= nil)
  t.eq(command.command, "rereview")
  t.is_true(tostring(response_body):find('outcome="applied"', 1, true) ~= nil)

  local response = trusted_comment(response_body, "IC_operator_response_rereview")
  t.eq(operator_commands.has_operator_command_response(comments, command), false)
  t.eq(operator_commands.has_operator_command_response(append_comment(comments, response), command), true)

  response.author_login = "untrusted-user"
  t.eq(operator_commands.has_operator_command_response(append_comment(comments, response), command), false)
  return {
    command = command.command,
    requires_applied_certificate = true,
    resolver = "operator_commands",
  }
end

local function stalled_review_markers(state_version)
  local review_proposal = devloop_base.pr_review_proposal_id(
    "owner/repo",
    pr_number,
    state_version,
    head_sha
  )
  local review_version = transition_version.safe_version_segment(state_version)
  local source_digest = convergence_shared.source_ref_digest({
    kind = "external",
    ref = "owner/repo#pr/7",
  })
  local angle_digests = {
    { angle = "minimal", verdict = "abstain", digest = "same-review-digest" },
  }
  return {
    conv_rounds.review_converge_round_marker(core, review_proposal, proposal_id, review_version,
      head_sha, source_digest, 1, "base", "Same review question", angle_digests),
    conv_rounds.review_converge_round_marker(core, review_proposal, proposal_id, review_version,
      head_sha, source_digest, 2, "loop1", "Same review question", angle_digests),
    conv_rounds.review_converge_round_marker(core, review_proposal, proposal_id, review_version,
      head_sha, source_digest, 3, "loop2", "Same review question", angle_digests),
  }
end

local function observe_rereview(source_state, state_version, id, extra_comments)
  local comments = {
    m_builders.pr_origin_marker(proposal_id, "42", branch, version, base_branch),
    core.state_marker(proposal_id, source_state, state_version),
  }
  for _, comment in ipairs(extra_comments or {}) do
    table.insert(comments, comment)
  end
  table.insert(comments, trusted_command(id))

  local source = core.current_state(comments, proposal_id)
  t.eq(source.state, source_state)
  h.mock_pr_origin(comments, branch, head_sha)
  local result = h.run_observe_pr(pr_event(), h.opts("restart-operator-reentry-" .. source_state))
  assert_department_ok(result, source_state)

  local response = find_accepted_response(result.raises)
  t.is_true(response ~= nil)
  local emitted = core.current_state({ trusted_comment(response.payload.body) }, proposal_id)
  t.eq(emitted.state, "reviewing")
  t.eq(emitted.version, operator_commands.operator_rereview_version(source.version, head_sha))
  t.is_true(emitted.version ~= source.version)

  return {
    owner = owner,
    kind = "operator_reentry",
    source = { state = source.state, boundary = nil },
    target = emitted.state,
    cause_evidence = applied_cause_evidence(comments, response.payload.body),
  }
end

local function edge_key(edge)
  local source = edge.source or {}
  local cause = edge.cause_evidence or {}
  return table.concat({
    tostring(edge.owner),
    tostring(edge.kind),
    tostring(source.state),
    tostring(source.boundary),
    tostring(edge.target),
    tostring(cause.command),
    tostring(cause.requires_applied_certificate),
    tostring(cause.resolver),
  }, "|")
end

local function assert_symmetric_edge_sets(observed, authored)
  local observed_keys = {}
  for _, edge in ipairs(observed) do
    local key = edge_key(edge)
    if observed_keys[key] then
      error("restart operator reentry conformance: duplicate production edge: " .. key)
    end
    observed_keys[key] = true
  end

  local authored_keys = {}
  for _, edge in ipairs(authored) do
    local key = edge_key(edge)
    if authored_keys[key] then
      error("restart operator reentry conformance: duplicate authored edge: " .. key)
    end
    authored_keys[key] = true
    if not observed_keys[key] then
      error("restart operator reentry conformance: authored edge was not observed in production: " .. key)
    end
  end
  for key in pairs(observed_keys) do
    if not authored_keys[key] then
      error("restart operator reentry conformance: production edge is missing from authored inventory: " .. key)
    end
  end
end

local function assert_operator_reentry_shape(edges)
  local seen_ids = {}
  local source_states = { blocked = true, ["review-meta"] = true, reviewing = true }
  for _, edge in ipairs(edges) do
    local edge_keys = key_set(structural_fields)
    edge_keys.pending_order = true
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { state = true })
    assert_exact_keys(edge.cause_evidence, {
      command = true,
      requires_applied_certificate = true,
      resolver = true,
    })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.row_id, "reviewing")
    t.eq(edge.kind, "operator_reentry")
    t.eq(source_states[edge.source.state], true)
    t.eq(edge.source.boundary, nil)
    t.eq(edge.target, "reviewing")
    t.eq(edge.cause_evidence.command, "rereview")
    t.eq(edge.cause_evidence.requires_applied_certificate, true)
    t.eq(edge.cause_evidence.resolver, "operator_commands")
    t.eq(edge.provenance.owner, owner)
    t.eq(edge.provenance.row, "reviewing")
    assert_same_value(edge.pending_order, pending_order_goldens[edge.id])
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function valid_operator_reentry()
  return {
    id = "owner/reviewing/operator_reentry/rereview_blocked",
    owner = "owner",
    row_id = "reviewing",
    kind = "operator_reentry",
    source = { state = "blocked", boundary = nil },
    target = "reviewing",
    cause_evidence = {
      command = "rereview",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "owner",
      row = "reviewing",
      field = "operator_reentry_inventory.rereview_blocked",
    },
  }
end

local function assert_extract_fails(selected_owner, inventory)
  local ok = pcall(function()
    restart_edges.extract_operator_reentry_edges(selected_owner, inventory)
  end)
  t.eq(ok, false)
end

local function assert_active_reviewing_is_not_accepted()
  local command = trusted_command("IC_rereview_active_reviewing_non_edge")
  local comments = {
    m_builders.pr_origin_marker(proposal_id, "42", branch, version, base_branch),
    core.state_marker(proposal_id, "reviewing", version),
    command,
  }
  h.mock_pr_origin(comments, branch, head_sha)
  local result = h.run_observe_pr(pr_event("2026-06-04T03:02:00Z"), h.opts("restart-operator-negative-active-reviewing"))
  assert_department_ok(result, "active-reviewing-negative")
  local response = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
  t.is_true(response ~= nil)
  t.is_true(tostring(response.payload.body):find('outcome="refused"', 1, true) ~= nil)
  t.eq(tostring(response.payload.body):find('outcome="applied"', 1, true), nil)
  t.eq(core.current_state({ trusted_comment(response.payload.body) }, proposal_id).state, nil)
end

local function assert_observed_inventory_edge(index, observed)
  local snapshot = copy_value(operator_reentry_inventory)
  local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
  assert_operator_reentry_shape(authored)
  t.eq(#authored, 3)
  assert_symmetric_edge_sets({ observed }, { authored[index] })
  assert_same_value(operator_reentry_inventory, snapshot)
end

return {
  test_pr_blocked_operator_reentry_matches_fresh_reviewing_marker = function()
    assert_observed_inventory_edge(1,
      observe_rereview("blocked", version .. "/review-loop/3", "IC_rereview_blocked"))
  end,

  test_pr_review_meta_operator_reentry_matches_fresh_reviewing_marker = function()
    assert_observed_inventory_edge(2,
      observe_rereview("review-meta", version .. "/review-meta-action/1", "IC_rereview_review_meta"))
  end,

  test_pr_reviewing_operator_reentry_matches_fresh_bumped_reviewing_marker = function()
    assert_observed_inventory_edge(3,
      observe_rereview("reviewing", version, "IC_rereview_stalled_reviewing", stalled_review_markers(version)))
  end,

  test_pr_operator_reentry_inventory_is_ordered_immutable_and_deterministic = function()
    local snapshot = copy_value(operator_reentry_inventory)
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)

    assert_operator_reentry_shape(authored)
    assert_same_value(operator_reentry_inventory, snapshot)
    t.eq(authored[1].id, "github-devloop-pr/reviewing/operator_reentry/rereview_blocked")
    t.eq(authored[2].id, "github-devloop-pr/reviewing/operator_reentry/rereview_review_meta")
    t.eq(authored[3].id, "github-devloop-pr/reviewing/operator_reentry/rereview_reviewing")

    local repeated = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_operator_reentry_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.cause_evidence ~= repeated[index].cause_evidence)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_pr_active_reviewing_refusal_does_not_satisfy_applied_reentry_cause = function()
    assert_active_reviewing_is_not_accepted()
  end,

  test_operator_reentry_edge_extractor_fails_closed_on_invalid_inventory = function()
    assert_extract_fails("", { valid_operator_reentry() })
    assert_extract_fails("owner", nil)
    assert_extract_fails("owner", { "not-an-edge" })

    local edge = valid_operator_reentry()
    edge.id = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.row_id = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.kind = "entry"
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.source = nil
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.source.state = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.source.boundary = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.target = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.cause_evidence = nil
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.cause_evidence.command = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.cause_evidence.requires_applied_certificate = false
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.cause_evidence.resolver = "other-resolver"
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.provenance = nil
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.provenance.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.provenance.row = ""
    assert_extract_fails("owner", { edge })

    edge = valid_operator_reentry()
    edge.provenance.field = ""
    assert_extract_fails("owner", { edge })

    assert_extract_fails("owner", { valid_operator_reentry(), valid_operator_reentry() })
  end,
}
