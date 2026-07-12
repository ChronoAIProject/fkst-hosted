local entity_lib = require("devloop.entity")
local entry_inventory = require("core.restart.entry_inventory")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local owner = "github-devloop-pr"
local proposal_id = "github-devloop/issue/owner/repo/42"
local pr_number = 7
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"
local base_branch = "dev"
local structural_fields = {
  "id",
  "owner",
  "row_id",
  "kind",
  "source",
  "target",
  "provenance",
}

local expected_entries = {
  ["github-devloop-pr/reviewing/entry/first_seen_pr"] = {
    row_id = "reviewing",
    output_variant = "first_seen_pr",
    source_state = nil,
    source_boundary = "github-proxy.github_entity_changed",
    target = "reviewing",
    field = "entry_inventory.first_seen_pr",
    cas_policy_id = "cas.legacy_observe_pr_v1",
    cas_variant = "pr_open_to_reviewing",
  },
  ["github-devloop-pr/reviewing/entry/review_receiver"] = {
    row_id = "reviewing",
    output_variant = "review_receiver",
    source_state = nil,
    source_boundary = "github-devloop-pr.devloop_reviewing",
    target = "reviewing",
    field = "entry_inventory.review_receiver",
    cas_policy_id = "cas.legacy_review_activation_handoff_v1",
  },
  ["github-devloop-pr/reviewing/entry/review_convergence_round"] = {
    row_id = "reviewing",
    output_variant = "review_convergence_round",
    source_state = nil,
    source_boundary = "consensus.consensus_converge",
    target = "reviewing",
    field = "entry_inventory.review_convergence_round",
    cas_policy_id = "cas.legacy_review_loop_safe_v1",
  },
  ["github-devloop-pr/pr-open/entry/pr_open_handoff"] = {
    row_id = "pr-open",
    output_variant = "pr_open_handoff",
    source_state = nil,
    source_boundary = "github-proxy.github_comment_written",
    target = "pr-open",
    field = "entry_inventory.pr_open_handoff",
  },
  ["github-devloop-pr/merge-ready/entry/handoff_to_merge_gate"] = {
    row_id = "merge-ready",
    output_variant = "handoff_to_merge_gate",
    source_state = "merge-ready",
    source_boundary = nil,
    target = "merging",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_merge_v1",
    cas_variant = "merge_ready_or_merging_to_merging",
  },
  ["github-devloop-pr/reviewing/entry/review_reject_to_blocked"] = {
    row_id = "reviewing",
    output_variant = "review_reject_to_blocked",
    source_state = "reviewing",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "review_reject_to_blocked",
  },
  ["github-devloop-pr/fixing/entry/review_reject_to_blocked"] = {
    row_id = "fixing",
    output_variant = "review_reject_to_blocked",
    source_state = "fixing",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "review_reject_to_blocked",
  },
  ["github-devloop-pr/fixing/entry/bounded_fix_to_blocked"] = {
    row_id = "fixing",
    output_variant = "bounded_fix_to_blocked",
    source_state = "fixing",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "bounded_fix_to_blocked",
  },
  ["github-devloop-pr/merge-ready/entry/review_reject_to_blocked"] = {
    row_id = "merge-ready",
    output_variant = "review_reject_to_blocked",
    source_state = "merge-ready",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "review_reject_to_blocked",
  },
  ["github-devloop-pr/merge-ready/entry/bounded_fix_to_blocked"] = {
    row_id = "merge-ready",
    output_variant = "bounded_fix_to_blocked",
    source_state = "merge-ready",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "bounded_fix_to_blocked",
  },
  ["github-devloop-pr/merging/entry/review_reject_to_blocked"] = {
    row_id = "merging",
    output_variant = "review_reject_to_blocked",
    source_state = "merging",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "review_reject_to_blocked",
  },
  ["github-devloop-pr/merging/entry/bounded_fix_to_blocked"] = {
    row_id = "merging",
    output_variant = "bounded_fix_to_blocked",
    source_state = "merging",
    source_boundary = "devloop_fix_reconcile",
    target = "blocked",
    field = "receiver_activations",
    cas_policy_id = "cas.legacy_pr_fix_reconcile_v1",
    cas_variant = "bounded_fix_to_blocked",
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

local function consumed_queue(spec, expected)
  for _, queue_name in ipairs(spec.consumes or {}) do
    if queue_name == expected then
      return queue_name
    end
  end
  error("restart entry conformance: production ingress queue is not declared in M.spec.consumes: " .. expected)
end

local function durable_queue_name(queue_name)
  if queue_name:find(".", 1, true) ~= nil then
    return queue_name
  end
  return owner .. "." .. queue_name
end

local function assert_department_ok(result, site)
  if result.exit_code ~= 0 then
    error("restart entry conformance: " .. site .. " failed: "
      .. tostring(result.error or result.stderr or "unknown department failure"))
  end
end

local function trusted_state_from_body(body, expected_state)
  local authored = {
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:02:05Z",
  }
  local emitted = core.current_state({ authored }, proposal_id)
  t.eq(emitted.state, expected_state)

  authored.author_login = "untrusted-user"
  t.eq(core.current_state({ authored }, proposal_id).state, nil)
  return emitted.state
end

local function pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = pr_number,
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
    source_ref = entity_lib.pr_source_ref("owner/repo", pr_number),
  }
end

local function origin_marker()
  return m_builders.pr_origin_marker(proposal_id, "42", branch, version, base_branch)
end

local function observe_first_seen_pr_entry()
  local department = require("departments.observe_pr.main")
  local queue_name = consumed_queue(department.spec, "github-proxy.github_entity_changed")

  t.eq(core.current_state({ origin_marker() }, proposal_id).state, nil)
  h.mock_bot_env()
  h.mock_default_issue_claim("owner/repo", 42)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = "owner/repo",
    number = pr_number,
    comments = { origin_marker() },
    head = branch,
    head_sha = "def456",
    state = "OPEN",
    base_branch = base_branch,
    labels = {},
  }, entity_read_mocks.pr_origin_selector)
  local result = h.run_department("departments/observe_pr/main.lua", {
    queue = queue_name,
    payload = pr_event(),
    ts = "2026-06-04T01:02:04Z",
  }, h.opts("restart-entry-observe-first-seen-pr"))

  assert_department_ok(result, "observe-first-seen-pr")
  local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
  t.is_true(comment ~= nil)
  t.eq(comment.payload.handoff.kind, "github-devloop.reviewing")
  t.eq(comment.payload.handoff.proposal_id, proposal_id)
  t.eq(comment.payload.handoff.pr_number, pr_number)
  t.eq(comment.payload.handoff.version, version)

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = trusted_state_from_body(comment.payload.body, "reviewing"),
  }, comment.payload
end

local function mock_marker_comment(comment_id, body)
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. tostring(comment_id) .. "'", {
    stdout = '{"id":"' .. h.json_string(comment_id)
      .. '","body":"' .. h.json_string(body)
      .. '","user":{"login":"fkst-test-bot"},"created_at":"2026-06-03T01:00:00Z"}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function comment_written_payload(request, comment_id)
  return {
    schema = "github-proxy.comment-written.v1",
    repo = request.repo,
    target = "pr",
    pr_number = request.pr_number,
    comment_id = comment_id,
    request_dedup_key = request.dedup_key,
    dedup_key = tostring(request.dedup_key) .. "/written/" .. tostring(comment_id),
    source_ref = request.source_ref,
    handoff = request.handoff,
  }
end

local function review_receiver_entry(reviewing_request)
  local handoff_department = require("departments.comment_handoff.main")
  local handoff_queue = consumed_queue(handoff_department.spec, "github-proxy.github_comment_written")
  local comment_id = "IC_restart_entry_reviewing_1"

  h.mock_default_issue_claim("owner/repo", 42)
  mock_marker_comment(comment_id, reviewing_request.body)
  local handoff_result = h.run_department("departments/comment_handoff/main.lua", {
    queue = handoff_queue,
    payload = comment_written_payload(reviewing_request, comment_id),
    ts = "2026-06-04T01:02:05Z",
  }, h.opts("restart-entry-reviewing-comment-handoff"))

  assert_department_ok(handoff_result, "reviewing-comment-handoff")
  local reviewing = h.find_raise(handoff_result.raises, "devloop_reviewing")
  t.is_true(reviewing ~= nil)

  local department = require("departments.review_pr.main")
  local queue_name = consumed_queue(department.spec, "devloop_reviewing")
  t.eq(reviewing.queue, queue_name)

  h.mock_issue_review({ "fkst-dev:reviewing" }, {
    core.state_marker(proposal_id, "reviewing", version),
  }, {
    title = "Implement decision recorder",
    body = "Issue context",
  })
  h.mock_pr_origin_sequence({
    { head = branch, head_sha = "def456" },
  })
  mock_marker_comment(comment_id, reviewing_request.body)
  local review_result = h.run_review_pr(reviewing.payload, h.opts("restart-entry-reviewing-receiver"))
  assert_department_ok(review_result, "reviewing-receiver")
  t.is_true(h.find_raise(review_result.raises, "consensus.proposal") ~= nil)

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = trusted_state_from_body(reviewing_request.body, "reviewing"),
  }
end

local function review_convergence_round_entry()
  local department = require("departments.review_loop.main")
  local queue_name = consumed_queue(department.spec, "consensus.consensus_converge")
  local reviewing_state = h.reviewing()
  local unresolved = h.review_unresolved({
    round = 0,
    narrowed_question = "Which reviewed-head evidence resolves the gap?",
    angle_digests = {
      { angle = "minimal", verdict = "comment", digest = "restart-entry-review-digest" },
    },
    findings_record = "open:\nreview finding remains unresolved",
  })
  local state_body = core.state_marker(proposal_id, "reviewing", reviewing_state.version)

  h.mock_bot_env()
  h.mock_pr_origin({
    origin_marker(),
    state_body,
  }, branch, "def456")
  h.mock_issue_review({ "fkst-dev:reviewing" }, { state_body }, {
    title = "Implement decision recorder",
    body = "Issue context",
  })
  local result = h.run_review_loop(unresolved, h.opts("restart-entry-review-convergence-round"))

  assert_department_ok(result, "review-convergence-round")
  t.is_true(h.find_raise(result.raises, "consensus.proposal") ~= nil)
  local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
  t.is_true(comment ~= nil)
  t.is_true(comment.payload.body:find("fkst:github-devloop:review-converge-round:v1", 1, true) ~= nil)
  t.eq(comment.payload.body:find("fkst:github-devloop:state:v1", 1, true), nil)

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = trusted_state_from_body(state_body, "reviewing"),
  }
end

local function pr_open_comment_body()
  return "github-devloop PR child open"
    .. "\n\n" .. origin_marker()
    .. "\n" .. m_builders.pr_link_marker(proposal_id, pr_number, branch, version, base_branch)
    .. "\n" .. core.state_marker(proposal_id, "pr-open", version)
end

local function pr_open_handoff_entry()
  local department = require("departments.comment_handoff.main")
  local queue_name = consumed_queue(department.spec, "github-proxy.github_comment_written")
  local comment_id = "IC_restart_entry_pr_open_1"
  local source_ref = entity_lib.pr_source_ref("owner/repo", pr_number)
  local body = pr_open_comment_body()

  mock_marker_comment(comment_id, body)
  local result = h.run_department("departments/comment_handoff/main.lua", {
    queue = queue_name,
    payload = {
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = pr_number,
      comment_id = comment_id,
      request_dedup_key = "pr-delegation/pr-open/" .. proposal_id .. "/g1",
      dedup_key = "pr-delegation/pr-open/" .. proposal_id .. "/g1/written/" .. comment_id,
      source_ref = source_ref,
      handoff = {
        kind = "github-devloop.pr_open",
        proposal_id = proposal_id,
        pr_number = pr_number,
        version = version,
        source_ref = source_ref,
      },
    },
    ts = "2026-06-04T01:02:05Z",
  }, h.opts("restart-entry-pr-open-handoff"))

  assert_department_ok(result, "pr-open-comment-handoff")
  local observe = h.find_raise(result.raises, "devloop_observe_pr")
  t.is_true(observe ~= nil)
  t.eq(observe.payload.schema, "github-proxy.v1")
  t.eq(observe.payload.type, "pr")
  t.eq(observe.payload.repo, "owner/repo")
  t.eq(observe.payload.number, pr_number)

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = trusted_state_from_body(body, "pr-open"),
  }
end

local function edge_key(edge)
  local source = edge.source or edge
  return table.concat({
    tostring(edge.owner),
    tostring(edge.kind),
    tostring(source.boundary),
    tostring(edge.target),
  }, "|")
end

local function assert_symmetric_edge_sets(observed, authored)
  local observed_keys = {}
  for _, edge in ipairs(observed) do
    local key = edge_key(edge)
    if observed_keys[key] then
      error("restart entry conformance: duplicate production ingress: " .. key)
    end
    observed_keys[key] = true
  end

  local authored_keys = {}
  for _, edge in ipairs(authored) do
    local key = edge_key(edge)
    if authored_keys[key] then
      error("restart entry conformance: duplicate authored entry edge: " .. key)
    end
    authored_keys[key] = true
    if not observed_keys[key] then
      error("restart entry conformance: authored entry edge was not observed in production: " .. key)
    end
  end
  for key in pairs(observed_keys) do
    if not authored_keys[key] then
      error("restart entry conformance: production ingress is missing from authored inventory: " .. key)
    end
  end
end

local function ingress_edges(edges)
  local out = {}
  for _, edge in ipairs(edges) do
    if edge.source.state == nil then
      table.insert(out, edge)
    end
  end
  return out
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

local function assert_entry_shape(edges)
  local seen_ids = {}
  for _, edge in ipairs(edges) do
    local expected = expected_entries[edge.id]
    t.is_true(expected ~= nil)
    local edge_keys = key_set(structural_fields)
    if expected.cas_policy_id ~= nil then
      edge_keys.cas_policy_id = true
    end
    if expected.cas_variant ~= nil then
      edge_keys.cas_variant = true
    end
    assert_exact_keys(edge, edge_keys)
    if expected.source_state == nil then
      assert_exact_keys(edge.source, { boundary = true })
    elseif expected.source_boundary ~= nil then
      assert_exact_keys(edge.source, { state = true, boundary = true })
    else
      assert_exact_keys(edge.source, { state = true })
    end
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.row_id, expected.row_id)
    t.eq(edge.kind, "entry")
    t.eq(edge.id, owner .. "/" .. expected.row_id .. "/entry/" .. expected.output_variant)
    t.eq(edge.source.state, expected.source_state)
    t.eq(edge.source.boundary, expected.source_boundary)
    t.eq(edge.target, expected.target)
    t.eq(edge.provenance.owner, owner)
    t.eq(edge.provenance.row, expected.row_id)
    t.eq(edge.provenance.field, expected.field)
    t.eq(edge.cas_policy_id, expected.cas_policy_id)
    t.eq(edge.cas_variant, expected.cas_variant)
    assert_valid_cas(edge)
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
  local expected_count = 0
  for id in pairs(expected_entries) do
    expected_count = expected_count + 1
    t.eq(seen_ids[id], true)
  end
  t.eq(#edges, expected_count)
end

local function valid_entry()
  return {
    id = "owner/reviewing/entry/site",
    owner = "owner",
    row_id = "reviewing",
    kind = "entry",
    source = { state = nil, boundary = "owner.queue" },
    target = "reviewing",
    provenance = {
      owner = "owner",
      row = "reviewing",
      field = "entry_inventory.site",
    },
  }
end

local function assert_extract_fails(selected_owner, inventory, rows)
  local ok = pcall(function()
    restart_edges.extract_entry_edges(selected_owner, inventory, rows)
  end)
  t.eq(ok, false)
end

return {
  test_pr_entry_inventory_matches_production_ingress_paths_symmetrically = function()
    local first_seen, reviewing_request = observe_first_seen_pr_entry()
    local observed = {
      first_seen,
      review_receiver_entry(reviewing_request),
      review_convergence_round_entry(),
      pr_open_handoff_entry(),
    }
    local snapshot = copy_value(entry_inventory)
    local rows = core.restart_transition_table()
    local rows_snapshot = copy_value(rows)
    local authored = restart_edges.extract_entry_edges(owner, entry_inventory, rows)

    assert_entry_shape(authored)
    assert_symmetric_edge_sets(observed, ingress_edges(authored))
    assert_same_value(entry_inventory, snapshot)
    assert_same_value(rows, rows_snapshot)
    t.eq(authored[1].id, "github-devloop-pr/reviewing/entry/first_seen_pr")
    t.eq(authored[2].id, "github-devloop-pr/reviewing/entry/review_receiver")
    t.eq(authored[3].id, "github-devloop-pr/reviewing/entry/review_convergence_round")
    t.eq(authored[4].id, "github-devloop-pr/pr-open/entry/pr_open_handoff")
    t.eq(authored[5].id, "github-devloop-pr/fixing/entry/review_reject_to_blocked")
    t.eq(authored[6].id, "github-devloop-pr/fixing/entry/bounded_fix_to_blocked")
    t.eq(authored[7].id, "github-devloop-pr/merge-ready/entry/handoff_to_merge_gate")
    t.eq(authored[8].id, "github-devloop-pr/merge-ready/entry/review_reject_to_blocked")
    t.eq(authored[9].id, "github-devloop-pr/merge-ready/entry/bounded_fix_to_blocked")
    t.eq(authored[10].id, "github-devloop-pr/merging/entry/review_reject_to_blocked")
    t.eq(authored[11].id, "github-devloop-pr/merging/entry/bounded_fix_to_blocked")
    t.eq(authored[12].id, "github-devloop-pr/reviewing/entry/review_reject_to_blocked")

    local repeated = restart_edges.extract_entry_edges(owner, entry_inventory, rows)
    assert_entry_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_entry_edge_extractor_fails_closed_on_invalid_inventory = function()
    assert_extract_fails("", { valid_entry() }, {})
    assert_extract_fails("owner", nil, {})
    assert_extract_fails("owner", { valid_entry() }, nil)
    assert_extract_fails("owner", { "not-an-edge" }, {})

    local edge = valid_entry()
    edge.id = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.owner = "other-owner"
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.row_id = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.kind = "autonomous"
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.source.state = "unmanaged"
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.source.boundary = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.target = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.provenance.owner = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.provenance.owner = "other-owner"
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.provenance.row = ""
    assert_extract_fails("owner", { edge }, {})

    edge = valid_entry()
    edge.provenance.field = ""
    assert_extract_fails("owner", { edge }, {})

    assert_extract_fails("owner", { valid_entry(), valid_entry() }, {})
  end,
}
