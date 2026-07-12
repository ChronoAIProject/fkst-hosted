local base_ids = require("devloop.base_ids")
local convergence_shared = require("devloop.convergence.shared")
local conv_reconcile = require("devloop.convergence.reconcile")
local conv_rounds = require("devloop.convergence.rounds")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local entry_inventory = require("core.restart.entry_inventory")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local operator_commands = require("devloop.operator_commands")
local operator_reentry_inventory = require("core.restart.operator_reentry_inventory")
local payloads_builders = require("devloop.payloads.builders")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local owner = "github-devloop"
local proposal_id = "github-devloop/issue/owner/repo/42"
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
local blocked_open_pr_id =
  "github-devloop/implementing/operator_reentry/reimplement_blocked_open_pr"
local blocked_timeout_without_pr_id =
  "github-devloop/implementing/operator_reentry/reimplement_blocked_implementing_timeout_without_pr"
local cas_metadata_golden = {
  [blocked_open_pr_id] = {
    cas_policy_id = "cas.legacy_implement_activation_handoff_v1",
    cas_variant = "blocked_to_implementing",
  },
  [blocked_timeout_without_pr_id] = {
    cas_policy_id = "cas.legacy_implement_activation_handoff_v1",
    cas_variant = "blocked_to_implementing",
  },
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

local function assert_valid_cas(edge)
  if edge.cas_policy_id == nil then
    return
  end
  local definition = restart_cas_catalog.definition(edge.cas_policy_id)
  t.is_true(definition ~= nil)
  if edge.cas_variant ~= nil then
    t.is_true(definition.variants ~= nil)
    t.is_true(definition.variants[edge.cas_variant] ~= nil)
  end
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

local function trusted_command(command, id)
  return trusted_comment("fkst: " .. command, id, "2026-06-04T03:00:00Z")
end

local function append_comment(comments, comment)
  local out = {}
  for _, current in ipairs(comments or {}) do
    table.insert(out, current)
  end
  table.insert(out, comment)
  return out
end

local function find_issue_comment(raises, needle)
  return h.find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(needle, 1, true) ~= nil
  end)
end

local function raises_summary(raises)
  local items = {}
  for _, raised in ipairs(raises or {}) do
    local body = tostring(raised.payload and raised.payload.body or "")
      :gsub("[\r\n]+", " ")
    if #body > 180 then
      body = body:sub(1, 180)
    end
    table.insert(items, tostring(raised.queue) .. "{" .. body .. "}")
  end
  return table.concat(items, ";")
end

local function assert_department_ok(result, site)
  if result.exit_code ~= 0 then
    error("restart operator reentry conformance: " .. site .. " failed: "
      .. tostring(result.error or result.stderr or "unknown department failure"))
  end
end

local function applied_cause_evidence(comments, command_name, response_body)
  local command = operator_commands.operator_command_fact(comments, command_name)
  t.is_true(command ~= nil, "applied cause: trusted command fact missing for " .. command_name)
  t.eq(command.command, command_name)
  t.is_true(tostring(response_body):find('outcome="applied"', 1, true) ~= nil,
    "applied cause: applied response certificate missing for " .. command_name)

  local response = trusted_comment(response_body, "IC_operator_response_" .. command_name)
  local with_response = append_comment(comments, response)
  t.eq(operator_commands.has_operator_command_response(comments, command), false)
  t.eq(operator_commands.has_operator_command_response(with_response, command), true)

  response.author_login = "untrusted-user"
  t.eq(operator_commands.has_operator_command_response(append_comment(comments, response), command), false)

  return {
    command = command.command,
    requires_applied_certificate = true,
    resolver = "operator_commands",
  }
end

local function row_by_state(state)
  for _, row in ipairs(core.restart_transition_table()) do
    if row.from_state == state then
      return row
    end
  end
  return nil
end

local function assert_existing_typed_edge(kind, source, target)
  local rows = core.restart_transition_table()
  local edges
  if kind == "entry" then
    edges = restart_edges.extract_entry_edges(owner, entry_inventory, rows)
  elseif kind == "guard_boundary" then
    edges = restart_edges.extract_guard_boundary_edges(owner, rows)
  else
    edges = restart_edges.extract_autonomous_edges(owner, rows)
  end
  for _, edge in ipairs(edges) do
    if edge.kind == kind and edge.source.state == source and edge.target == target then
      return
    end
  end
  error("restart operator reentry conformance: expected existing " .. kind .. " edge "
    .. source .. " -> " .. target)
end

local function mock_linked_pr_state(comments, state)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, h.render_comment(comment))
  end
  entity_read_mocks.mock_pr_view_raw_selector(t, {}, entity_read_mocks.pr_origin_selector, {
    stdout = string.format(
      '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"%s","updatedAt":"2026-06-03T02:03:04Z","comments":[%s]}\n',
      h.json_string(state or "OPEN"),
      table.concat(rendered_comments, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function run_reimplement_case(case)
  local source = core.current_state(case.comments, proposal_id)
  t.eq(source.state, case.source_state)
  h.mock_issue_state(case.labels, "OPEN", case.comments)
  if case.before_run ~= nil then
    case.before_run()
  end
  local result = h.run_observe(case.event, h.opts(case.name))
  assert_department_ok(result, case.name)

  local ready = h.find_raise(result.raises, "devloop_ready")
  local response = find_issue_comment(result.raises, "operator command accepted: reimplement")
  t.is_true(ready ~= nil, case.name .. ": devloop_ready was not emitted")
  t.is_true(response ~= nil, case.name .. ": applied response was not emitted")

  local target_row = row_by_state("implementing")
  t.is_true(target_row ~= nil, case.name .. ": implementing restart row is missing")
  t.eq(target_row.driving_queue, ready.queue)

  local boundary = nil
  local reentry = ready.payload.operator_reentry
  if source.state == "blocked" then
    t.is_true(type(reentry) == "table", case.name .. ": blocked reentry evidence is missing; ready="
      .. tostring(ready.payload.dedup_key) .. "; raises=" .. raises_summary(result.raises))
    t.eq(reentry.command, "reimplement")
    t.eq(reentry.from_state, "blocked")
    if reentry.pr_number ~= nil then
      boundary = "open-pr"
      t.eq(tonumber(reentry.pr_number), 7)
    else
      boundary = reentry.terminal_reason
      t.eq(boundary, "implementing-timeout-without-pr")
    end
  else
    t.eq(reentry, nil)
  end

  return {
    owner = owner,
    kind = "operator_reentry",
    source = { state = source.state, boundary = boundary },
    target = target_row.from_state,
    cause_evidence = applied_cause_evidence(case.comments, "reimplement", response.payload.body),
  }
end

local function observe_impl_failed_reimplement()
  local event = h.reached()
  local ready_version = payloads_builders.build_devloop_ready_payload(core, event).dedup_key
  return run_reimplement_case({
    name = "restart-operator-reentry-impl-failed",
    event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }),
    labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" },
    source_state = "impl-failed",
    comments = {
      core.state_marker(proposal_id, "impl-failed", ready_version),
      core.impl_failure_marker(proposal_id, ready_version, "codex-failed", 2),
      trusted_command("reimplement", "IC_reimplement_impl_failed"),
    },
  })
end

local function observe_blocked_open_pr_reimplement()
  local event = h.reached()
  local ready_version = payloads_builders.build_devloop_ready_payload(core, event).dedup_key
  local blocked_version = ready_version .. "/review-loop/3"
  return run_reimplement_case({
    name = "restart-operator-reentry-blocked-open-pr",
    event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } }),
    labels = { "fkst-dev:enabled", "fkst-dev:blocked" },
    source_state = "blocked",
    comments = {
      m_builders.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", ready_version, "dev"),
      core.state_marker(proposal_id, "blocked", blocked_version),
      trusted_command("reimplement", "IC_reimplement_blocked_open_pr"),
    },
    before_run = function()
      mock_linked_pr_state({}, "OPEN")
    end,
  })
end

local function observe_blocked_timeout_reimplement()
  local event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } })
  local inner_version = "github-devloop/issue/owner/repo/42/intake/2226"
  local ready_version = payloads_builders.build_devloop_ready_payload(core, {
    proposal_id = proposal_id,
    dedup_key = inner_version,
    source_ref = event.source_ref,
  }).dedup_key
  local blocked_version = conv_reconcile.timeout_reconcile_state_version(ready_version, "implementing", 3)
  return run_reimplement_case({
    name = "restart-operator-reentry-blocked-implementing-timeout",
    event = event,
    labels = { "fkst-dev:enabled", "fkst-dev:blocked" },
    source_state = "blocked",
    comments = {
      core.state_marker(proposal_id, "implementing", ready_version),
      core.state_marker(proposal_id, "blocked", blocked_version),
      conv_reconcile.timeout_reconcile_marker(proposal_id, ready_version, "implementing", 3, "drop", {
        terminal_version = blocked_version,
        from_state = "implementing",
        from_version = ready_version,
        reason_class = "state-output-obligation-timeout",
        source_ref = event.source_ref,
      }),
      trusted_command("reimplement", "IC_reimplement_blocked_timeout"),
    },
  })
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
  for _, edge in ipairs(edges) do
    local edge_keys = key_set(structural_fields)
    local expected_cas = cas_metadata_golden[edge.id]
    if expected_cas ~= nil then
      edge_keys.cas_policy_id = true
      edge_keys.cas_variant = true
    end
    assert_exact_keys(edge, edge_keys)
    if edge.source.boundary == nil then
      assert_exact_keys(edge.source, { state = true })
    else
      assert_exact_keys(edge.source, { state = true, boundary = true })
    end
    assert_exact_keys(edge.cause_evidence, {
      command = true,
      requires_applied_certificate = true,
      resolver = true,
    })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.row_id, "implementing")
    t.eq(edge.kind, "operator_reentry")
    t.eq(edge.target, "implementing")
    t.eq(edge.cause_evidence.command, "reimplement")
    t.eq(edge.cause_evidence.requires_applied_certificate, true)
    t.eq(edge.cause_evidence.resolver, "operator_commands")
    t.eq(edge.provenance.owner, owner)
    t.eq(edge.provenance.row, "implementing")
    t.eq(edge.cas_policy_id, expected_cas and expected_cas.cas_policy_id or nil)
    t.eq(edge.cas_variant, expected_cas and expected_cas.cas_variant or nil)
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function thinking_converge_comments(event, command)
  local base_version = payloads_builders.build_proposal(event).dedup_key
  local source_digest = convergence_shared.source_ref_digest(event.source_ref)
  local angle_digests = {
    { angle = "minimal", verdict = "abstain", digest = "same-digest" },
  }
  local comments = {
    core.state_marker(proposal_id, "thinking", base_version .. "/loop/7"),
  }
  for round = 1, 7 do
    table.insert(comments, conv_rounds.converge_round_marker(
      proposal_id,
      base_version,
      source_digest,
      round,
      base_version .. "/loop/" .. tostring(round),
      "Same narrowed question",
      angle_digests
    ))
  end
  table.insert(comments, command)
  return comments
end

local function observe_rereview_row_replay()
  local event = h.issue()
  local comments = thinking_converge_comments(event, trusted_command("rereview", "IC_negative_rereview"))
  local before = core.current_state(comments, proposal_id)
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", comments)
  local result = h.run_observe(event, h.opts("restart-operator-negative-rereview"))
  assert_department_ok(result, "negative-rereview")

  local proposal = h.find_raise(result.raises, "consensus.proposal")
  local response = find_issue_comment(result.raises, "operator command accepted: rereview")
  t.is_true(proposal ~= nil, "negative rereview: consensus proposal was not replayed")
  t.is_true(response ~= nil, "negative rereview: applied response was not emitted")
  t.eq(tostring(response.payload.body):find("fkst:github-devloop:state:v1", 1, true), nil)
  applied_cause_evidence(comments, "rereview", response.payload.body)

  local after = core.current_state(append_comment(comments, trusted_comment(response.payload.body)), proposal_id)
  t.eq(after.state, "thinking")
  t.eq(after.version, before.version)
  return { kind = "row-replay", command = "rereview", source_state = "thinking" }
end

local function observe_ready_reready_row_replay()
  local event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:ready" } })
  local ready_version = payloads_builders.build_devloop_ready_payload(core, h.reached()).dedup_key
  local comments = {
    trusted_comment(
      core.state_marker(proposal_id, "ready", ready_version, "result-marker,ready-label,devloop-ready"),
      "IC_ready_handoff"
    ),
    trusted_command("reready", "IC_negative_reready_ready"),
  }
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" }, "OPEN", comments)
  local result = h.run_observe(event, h.opts("restart-operator-negative-reready-ready"))
  assert_department_ok(result, "negative-reready-ready")

  local response = find_issue_comment(result.raises, "operator command accepted: reready")
  local ready = h.find_raise(result.raises, "devloop_ready")
  t.is_true(response ~= nil, "negative ready reready: applied response was not emitted; raises="
    .. raises_summary(result.raises))
  t.is_true(ready ~= nil, "negative ready reready: devloop_ready was not replayed")
  t.eq(ready.payload.ready_hand_off.marker_version, ready_version)
  t.eq(tostring(response.payload.body):find("fkst:github-devloop:state:v1", 1, true), nil)
  applied_cause_evidence(comments, "reready", response.payload.body)
  assert_existing_typed_edge("entry", "ready", "implementing")
  return { kind = "row-replay", command = "reready", source_state = "ready" }
end

local function observe_dependency_wait_reready_row_replay()
  local event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:ready" } })
  local version = "ready/consensus-github-devloop/issue/owner/repo/42/dependency"
  local comments = {
    core.state_marker(proposal_id, "dependency_wait", version),
    "github-devloop dependency hold: unresolvable\n\n"
      .. core.dependency_unresolvable_marker(proposal_id, version, { 42 }),
    trusted_command("reready", "IC_negative_reready_dependency_wait"),
  }
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" }, "OPEN", comments)
  local result = h.run_observe(event, h.opts("restart-operator-negative-reready-dependency-wait"))
  assert_department_ok(result, "negative-reready-dependency-wait")

  local response = find_issue_comment(result.raises, "operator command accepted: reready")
  local ready_comment = find_issue_comment(result.raises, 'state="ready"')
  t.is_true(response ~= nil, "negative dependency_wait reready: applied response was not emitted")
  t.is_true(ready_comment ~= nil, "negative dependency_wait reready: ready marker was not emitted")
  applied_cause_evidence(comments, "reready", response.payload.body)
  local emitted = core.current_state({ trusted_comment(ready_comment.payload.body) }, proposal_id)
  t.eq(emitted.state, "ready")
  assert_existing_typed_edge("guard_boundary", "dependency_wait", "ready")
  return { kind = "row-replay", command = "reready", source_state = "dependency_wait" }
end

local function observe_blocked_reready_row_replay()
  local event = h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } })
  local ready_version = "consensus:github-devloop/issue/owner/repo/42/intake/1116/loop/1"
  local blocked_version = conv_reconcile.timeout_reconcile_state_version(ready_version, "ready", 3)
  local comments = {
    trusted_comment(
      core.state_marker(proposal_id, "ready", ready_version, "result-marker,ready-label,devloop-ready"),
      "IC_ready_before_timeout"
    ),
    core.state_marker(proposal_id, "blocked", blocked_version),
    conv_reconcile.timeout_reconcile_marker(proposal_id, ready_version, "ready", 3, "drop", {
      terminal_version = blocked_version,
      from_state = "ready",
      from_version = ready_version,
      source_ref = event.source_ref,
    }),
    trusted_command("reready", "IC_negative_reready_blocked"),
  }
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:blocked" }, "OPEN", comments)
  local result = h.run_observe(event, h.opts("restart-operator-negative-reready-blocked"))
  assert_department_ok(result, "negative-reready-blocked")

  local response = find_issue_comment(result.raises, "operator command accepted: reready")
  local ready = h.find_raise(result.raises, "devloop_ready")
  t.is_true(response ~= nil, "negative blocked reready: applied response was not emitted")
  t.is_true(ready ~= nil, "negative blocked reready: devloop_ready was not replayed")
  t.eq(ready.payload.ready_hand_off.marker_version, ready_version)
  applied_cause_evidence(comments, "reready", response.payload.body)
  assert_existing_typed_edge("entry", "ready", "implementing")
  return { kind = "row-replay", command = "reready", source_state = "blocked" }
end

local function observe_reintake_admission_contract()
  local command_comment = trusted_command("reintake", "IC_negative_reintake")
  local active_comments = {
    core.state_marker(proposal_id, "thinking", "thinking/reintake-refused"),
    command_comment,
  }
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", active_comments)
  local observed = h.run_observe(
    h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:thinking" } }),
    h.opts("restart-operator-negative-reintake-observe-issue")
  )
  assert_department_ok(observed, "negative-reintake-observe-issue")
  t.eq(find_issue_comment(observed.raises, "operator command accepted: reintake"), nil)

  local blocked_comments = {
    core.state_marker(proposal_id, "blocked", "blocked/reintake-admission"),
    command_comment,
  }
  local command = operator_commands.operator_command_fact(blocked_comments, "reintake")
  t.is_true(command ~= nil, "negative reintake: trusted command fact missing")
  t.eq(operator_commands.has_operator_command_response(blocked_comments, command), false)

  t.eq(operator_commands.reintake_has_active_devloop_state(
    { "fkst-dev:enabled", "fkst-dev:blocked" },
    blocked_comments,
    proposal_id
  ), false)

  local response = operator_commands.build_operator_issue_reintake_comment_request(
    "owner/repo",
    42,
    command,
    { dedup_key = "intake-candidate/github-devloop/issue/owner/repo/42" },
    h.issue().source_ref
  )
  t.eq(tostring(response.body):find("fkst:github-devloop:state:v1", 1, true), nil)
  applied_cause_evidence(blocked_comments, "reintake", response.body)

  t.eq(operator_commands.reintake_has_active_devloop_state(
    { "fkst-dev:enabled", "fkst-dev:thinking" },
    active_comments,
    proposal_id
  ), true)
  return { kind = "admission", command = "reintake", source_state = nil }
end

local function assert_negative_witnesses_absent(witnesses, authored)
  for _, witness in ipairs(witnesses) do
    t.is_true(witness.kind == "row-replay" or witness.kind == "admission",
      "negative witness has an unexpected classification")
    for _, edge in ipairs(authored) do
      if edge.cause_evidence.command == witness.command then
        error("restart operator reentry conformance: non-edge command was authored: " .. witness.command)
      end
    end
  end
end

local function valid_operator_reentry()
  return {
    id = "owner/implementing/operator_reentry/reimplement_impl_failed",
    owner = "owner",
    row_id = "implementing",
    kind = "operator_reentry",
    source = { state = "impl-failed", boundary = nil },
    target = "implementing",
    cause_evidence = {
      command = "reimplement",
      requires_applied_certificate = true,
      resolver = "operator_commands",
    },
    provenance = {
      owner = "owner",
      row = "implementing",
      field = "operator_reentry_inventory.reimplement_impl_failed",
    },
  }
end

local function assert_extract_fails(selected_owner, inventory)
  local ok = pcall(function()
    restart_edges.extract_operator_reentry_edges(selected_owner, inventory)
  end)
  t.eq(ok, false)
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
  test_issue_impl_failed_operator_reentry_matches_production_apply_decision = function()
    assert_observed_inventory_edge(1, observe_impl_failed_reimplement())
  end,

  test_issue_blocked_open_pr_operator_reentry_matches_production_apply_decision = function()
    assert_observed_inventory_edge(2, observe_blocked_open_pr_reimplement())
  end,

  test_issue_blocked_timeout_operator_reentry_matches_production_apply_decision = function()
    assert_observed_inventory_edge(3, observe_blocked_timeout_reimplement())
  end,

  test_issue_blocked_operator_reentry_cas_metadata_references_declared_policies = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    local edges_by_id = {}
    for _, edge in ipairs(authored) do
      edges_by_id[edge.id] = edge
    end
    for _, id in ipairs({ blocked_open_pr_id, blocked_timeout_without_pr_id }) do
      local edge = edges_by_id[id]
      local expected = cas_metadata_golden[id]
      t.is_true(edge ~= nil)
      t.eq(edge.cas_policy_id, expected.cas_policy_id)
      t.eq(edge.cas_variant, expected.cas_variant)
      assert_valid_cas(edge)
    end
  end,

  test_issue_operator_reentry_inventory_is_ordered_immutable_and_deterministic = function()
    local snapshot = copy_value(operator_reentry_inventory)
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)

    assert_operator_reentry_shape(authored)
    assert_same_value(operator_reentry_inventory, snapshot)
    t.eq(authored[1].id, "github-devloop/implementing/operator_reentry/reimplement_impl_failed")
    t.eq(authored[2].id, "github-devloop/implementing/operator_reentry/reimplement_blocked_open_pr")
    t.eq(authored[3].id, "github-devloop/implementing/operator_reentry/reimplement_blocked_implementing_timeout_without_pr")

    local repeated = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_operator_reentry_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.cause_evidence ~= repeated[index].cause_evidence)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_issue_rereview_is_row_replay_not_operator_reentry = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_negative_witnesses_absent({ observe_rereview_row_replay() }, authored)
  end,

  test_issue_ready_reready_is_row_replay_not_operator_reentry = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_negative_witnesses_absent({ observe_ready_reready_row_replay() }, authored)
  end,

  test_issue_dependency_wait_reready_redrives_existing_edge_not_operator_reentry = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_negative_witnesses_absent({ observe_dependency_wait_reready_row_replay() }, authored)
  end,

  test_issue_blocked_reready_is_row_replay_not_operator_reentry = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_negative_witnesses_absent({ observe_blocked_reready_row_replay() }, authored)
  end,

  test_issue_reintake_is_admission_not_operator_reentry = function()
    local authored = restart_edges.extract_operator_reentry_edges(owner, operator_reentry_inventory)
    assert_negative_witnesses_absent({ observe_reintake_admission_contract() }, authored)
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
