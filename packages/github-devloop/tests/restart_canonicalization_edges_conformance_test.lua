local base_ids = require("devloop.base_ids")
local canonicalization_inventory = require("core.restart.canonicalization_inventory")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local restart_cas_catalog = require("devloop.restart_cas_catalog")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local owner = "github-devloop"
local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local proposal_id = "github-devloop/issue/owner/repo/42"
local pr_proposal_id = "github-devloop/pr/owner/repo/7"
local ready_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"
local base_branch = "dev"
local head_sha = "0123456789abcdef0123456789abcdef01234567"
local merge_commit_sha = "1111111111111111111111111111111111111111"
local structural_fields = {
  "id",
  "owner",
  "row_id",
  "kind",
  "source",
  "target",
  "semantic_variant",
  "cause_evidence",
  "provenance",
}
local implementing_merged_delegated_pr_id =
  "github-devloop/awaiting-pr/canonicalization/implementing_merged_delegated_pr"
local cas_metadata_golden = {
  [implementing_merged_delegated_pr_id] = {
    cas_policy_id = "cas.legacy_awaiting_pr_v1",
    cas_variant = "implementing_to_awaiting_pr",
  },
}
local pending_order_goldens = {
  ["github-devloop/dependency_wait/canonicalization/legacy_ready_dependency_hold"] = { participates = true, predecessor_state = "ready" },
  ["github-devloop/ready/canonicalization/legacy_ready_rederive"] = { participates = false },
  [implementing_merged_delegated_pr_id] = { participates = true, predecessor_state = "implementing" },
  ["github-devloop/awaiting-pr/canonicalization/legacy_pr_open_delegation"] = { participates = false },
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

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T02:06:04Z",
  }
end

local function encode_json_string(value)
  return tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function render_comment(body)
  return string.format(
    '{"body":"%s","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-03T01:00:00Z"}',
    encode_json_string(body or "")
  )
end

local function issue_comments_json(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, render_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function issue_view_json(labels, comments)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', encode_json_string(label)))
  end
  return string.format(
    '{"title":"Canonicalization conformance","state":"OPEN","labels":[%s],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    table.concat(rendered_labels, ","),
    issue_comments_json(comments)
  )
end

local function blocked_by_json(nodes)
  local rendered = {}
  for _, node in ipairs(nodes or {}) do
    table.insert(rendered, string.format(
      '{"number":%s,"state":"%s","stateReason":"","repository":{"nameWithOwner":"%s"}}',
      tostring(node.number),
      tostring(node.state or "OPEN"),
      tostring(node.repo or repo)
    ))
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":'
    .. tostring(#(nodes or {}))
    .. ',"pageInfo":{"hasNextPage":false},"nodes":['
    .. table.concat(rendered, ",")
    .. ']}}}}}\n'
end

local function mock_issue_view(labels, comments)
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = issue_number,
    labels = labels,
    comments = comments,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    times = 1,
  })
  t.mock_command(core.gh_issue_view_entity_cmd(repo, issue_number), {
    stdout = issue_view_json(labels, comments),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocked_by(number, nodes)
  t.mock_command(core.gh_blocked_by_cmd(repo, number), {
    stdout = blocked_by_json(nodes),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocked_by_failure(number)
  t.mock_command(core.gh_blocked_by_cmd(repo, number), {
    stdout = "",
    stderr = "graphql failed",
    exit_code = 1,
  })
end

local function mock_blocker_issue(number, state)
  local blocker_proposal = base_ids.proposal_id(repo, number)
  t.mock_command(core.gh_issue_view_observe_cmd(repo, number), {
    stdout = '{"state":"OPEN","comments":['
      .. render_comment(core.state_marker(blocker_proposal, state, "v-" .. tostring(number)))
      .. '],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function run_issue_observe(name, payload)
  return h.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = payload or h.issue(),
  }, h.opts(name))
end

local function find_comment_with(raises, needle)
  return h.find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(needle, 1, true) ~= nil
  end)
end

local function assert_department_ok(result, site)
  if result.exit_code ~= 0 then
    error("restart canonicalization conformance: " .. site .. " failed: "
      .. tostring(result.error or result.stderr or "unknown department failure"))
  end
end

local function observed_edge(source_state, target_state, marker, resolver)
  return {
    owner = owner,
    kind = "canonicalization",
    source = { state = source_state, boundary = nil },
    target = target_state,
    cause_evidence = {
      marker = marker,
      resolver = resolver,
    },
  }
end

local function observe_ready_split(target)
  local labels = { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" }
  local comments
  if target == "ready" then
    comments = {
      core.state_marker(proposal_id, "ready", ready_version),
      "github-devloop dependency hold: waiting\n\nReason: waiting-on-dependency\n\n"
        .. core.dependency_wait_marker(proposal_id, ready_version, { 53 }),
    }
    mock_issue_view(labels, comments)
    mock_blocked_by(issue_number, { { number = 53 } })
    mock_blocked_by(53, {})
    mock_blocker_issue(53, "merged")
  else
    comments = {
      core.state_marker(proposal_id, "ready", ready_version),
      "github-devloop dependency hold: unresolvable\n\nReason: gh-failed\n\n"
        .. core.dependency_unresolvable_marker(proposal_id, ready_version, { issue_number }),
    }
    mock_issue_view(labels, comments)
    mock_blocked_by_failure(issue_number)
  end

  local source = core.current_state(comments, proposal_id)
  t.eq(source.state, "ready")
  local result = run_issue_observe("restart-canonicalization-ready-to-" .. target)
  assert_department_ok(result, "ready-to-" .. target)
  local emitted = find_comment_with(result.raises, "fkst:github-devloop:ready-split-canonicalized:v1")
  t.is_true(emitted ~= nil)
  local emitted_comments = { trusted_comment(emitted.payload.body) }
  local fact = core.ready_split_canonicalized_fact(emitted_comments, proposal_id, source.version)
  t.is_true(fact ~= nil)
  t.eq(fact.derived_state, target)
  local state = core.current_state(emitted_comments, proposal_id)
  t.eq(state.state, target)
  t.eq(state.version, fact.to_version)
  return observed_edge(source.state, state.state,
    "ready-split-canonicalized:v1", "ready_split_canonicalized_fact")
end

local function parent_comments(state)
  return {
    core.state_marker(proposal_id, state, impl_version),
    m_builders.pr_delegation_marker(proposal_id, pr_proposal_id, pr_number, impl_version, "g1"),
  }
end

local function mock_merged_child_reads()
  local issue_comments = parent_comments("implementing")
  local pr_comments = {
    m_builders.pr_origin_marker(proposal_id, issue_number, branch, impl_version, base_branch),
  }
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = issue_number,
    labels = { "fkst-dev:enabled", "fkst-dev:implementing" },
    comments = issue_comments,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    register_all_views = true,
    times = 1,
  })
  local pr_fields = {
    repo = repo,
    number = pr_number,
    comments = pr_comments,
    head = branch,
    head_sha = head_sha,
    merge_commit_sha = merge_commit_sha,
    state = "MERGED",
    merged_at = "2026-06-03T02:05:04Z",
    base_branch = base_branch,
    labels = {},
    register_all_views = true,
    times = 1,
  }
  entity_read_mocks.mock_pr_read_forms(t, pr_fields)
  entity_read_mocks.mock_pr_view_selector(t, pr_fields, entity_read_mocks.pr_origin_selector, 1)
  return issue_comments
end

local function observe_implementing_merged_child()
  h.mock_bot_env()
  h.mock_write_env("")
  t.mock_command("gh api graphql", {
    stdout = blocked_by_json({}),
    stderr = "",
    exit_code = 0,
  })
  local comments = mock_merged_child_reads()
  local source = core.current_state(comments, proposal_id)
  t.eq(source.state, "implementing")
  local result = run_issue_observe("restart-canonicalization-implementing-awaiting-pr", {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = pr_number,
    state = "MERGED",
    updated_at = "2026-06-03T02:03:04Z",
    dedup_key = "owner/repo#pr#7@2026-06-03T02:03:04Z",
    source_ref = entity_lib.pr_source_ref(repo, pr_number),
  })
  assert_department_ok(result, "implementing-to-awaiting-pr")
  local emitted = find_comment_with(result.raises, "fkst:github-devloop:pr-delegation:v1")
  t.is_true(emitted ~= nil)
  local emitted_comments = { trusted_comment(emitted.payload.body) }
  local delegation = m_facts.pr_delegation_fact(emitted_comments, proposal_id, source.version)
  t.is_true(delegation ~= nil)
  t.eq(delegation.pr_proposal_id, pr_proposal_id)
  local state = core.current_state(emitted_comments, proposal_id)
  t.eq(state.state, "awaiting-pr")
  return observed_edge(source.state, state.state, "pr-delegation:v1", "pr_delegation_fact")
end

local function observe_legacy_pr_open()
  local comments = {
    core.state_marker(proposal_id, "pr-open", impl_version),
    m_builders.pr_link_marker(proposal_id, pr_number, branch, impl_version, base_branch),
  }
  h.mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    comments = {
      h.render_comment(m_builders.pr_origin_marker(proposal_id, issue_number, branch, impl_version, base_branch)
        .. "\n" .. core.state_marker(proposal_id, "pr-open", impl_version)),
    },
    head = branch,
    head_sha = "def456",
    base_branch = base_branch,
    state = "OPEN",
  }, entity_read_mocks.pr_origin_selector)

  local source = core.current_state(comments, proposal_id)
  t.eq(source.state, "pr-open")
  local result = h.run_observe(
    h.issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }),
    h.opts("restart-canonicalization-legacy-pr-open")
  )
  assert_department_ok(result, "legacy-pr-open-to-awaiting-pr")
  local emitted = find_comment_with(result.raises, "fkst:github-devloop:pr-delegation:v1")
  t.is_true(emitted ~= nil)
  local emitted_comments = { trusted_comment(emitted.payload.body) }
  local delegation = m_facts.pr_delegation_fact(emitted_comments, proposal_id, source.version)
  t.is_true(delegation ~= nil)
  t.eq(delegation.pr_proposal_id, pr_proposal_id)
  local state = core.current_state(emitted_comments, proposal_id)
  t.eq(state.state, "awaiting-pr")
  return observed_edge(source.state, state.state, "pr-delegation:v1", "pr_delegation_fact")
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
    tostring(cause.marker),
    tostring(cause.resolver),
  }, "|")
end

local function assert_symmetric_edge_sets(observed, authored)
  local observed_keys = {}
  for _, edge in ipairs(observed) do
    local key = edge_key(edge)
    if observed_keys[key] then
      error("restart canonicalization conformance: duplicate production edge: " .. key)
    end
    observed_keys[key] = true
  end

  local authored_keys = {}
  for _, edge in ipairs(authored) do
    local key = edge_key(edge)
    if authored_keys[key] then
      error("restart canonicalization conformance: duplicate authored edge: " .. key)
    end
    authored_keys[key] = true
    if not observed_keys[key] then
      error("restart canonicalization conformance: authored edge was not observed in production: " .. key)
    end
  end
  for key in pairs(observed_keys) do
    if not authored_keys[key] then
      error("restart canonicalization conformance: production edge is missing from authored inventory: " .. key)
    end
  end
end

local function assert_canonicalization_shape(edges)
  local seen_ids = {}
  for _, edge in ipairs(edges) do
    local edge_keys = key_set(structural_fields)
    local expected_cas = cas_metadata_golden[edge.id]
    if expected_cas ~= nil then
      edge_keys.cas_policy_id = true
      edge_keys.cas_variant = true
    end
    edge_keys.pending_order = true
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { state = true })
    assert_exact_keys(edge.cause_evidence, { marker = true, resolver = true })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.kind, "canonicalization")
    t.eq(edge.source.boundary, nil)
    t.eq(edge.row_id, edge.target)
    t.eq(type(edge.semantic_variant), "string")
    t.is_true(edge.semantic_variant ~= "")
    t.eq(edge.semantic_variant:find("/", 1, true), nil)
    t.eq(edge.semantic_variant, edge.id:match("/([^/]+)$"))
    t.eq(edge.provenance.owner, owner)
    t.eq(edge.provenance.row, edge.target)
    t.eq(edge.cas_policy_id, expected_cas and expected_cas.cas_policy_id or nil)
    t.eq(edge.cas_variant, expected_cas and expected_cas.cas_variant or nil)
    assert_same_value(edge.pending_order, pending_order_goldens[edge.id])
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function valid_canonicalization(semantic_variant, source_state, target, marker, resolver)
  return {
    semantic_variant = semantic_variant or "legacy_ready_rederived",
    owner = "owner",
    row_id = target or "ready",
    kind = "canonicalization",
    source = {
      state = source_state or "ready",
      boundary = nil,
    },
    target = target or "ready",
    cause_evidence = {
      marker = marker or "ready-split-canonicalized:v1",
      resolver = resolver or "ready_split_canonicalized_fact",
    },
    provenance = {
      owner = "owner",
      row = target or "ready",
      field = "canonicalization_inventory.legacy_ready_rederived",
    },
  }
end

local function assert_extract_fails(selected_owner, inventory)
  local ok = pcall(function()
    restart_edges.extract_canonicalization_edges(selected_owner, inventory)
  end)
  t.eq(ok, false)
end

local function assert_observed_inventory_edge(index, observed)
  local snapshot = copy_value(canonicalization_inventory)
  local authored = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)
  assert_canonicalization_shape(authored)
  t.eq(#authored, 4)
  assert_symmetric_edge_sets({ observed }, { authored[index] })
  assert_same_value(canonicalization_inventory, snapshot)
end

return {
  test_issue_ready_dependency_hold_canonicalization_matches_production_marker = function()
    assert_observed_inventory_edge(1, observe_ready_split("dependency_wait"))
  end,

  test_issue_ready_rederive_canonicalization_matches_production_marker = function()
    assert_observed_inventory_edge(2, observe_ready_split("ready"))
  end,

  test_issue_implementing_handoff_canonicalization_matches_production_marker = function()
    assert_observed_inventory_edge(3, observe_implementing_merged_child())
  end,

  test_issue_implementing_handoff_canonicalization_references_declared_cas_policy = function()
    local edge
    for _, candidate in ipairs(restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)) do
      if candidate.id == implementing_merged_delegated_pr_id then
        edge = candidate
        break
      end
    end
    t.is_true(edge ~= nil)
    t.eq(edge.cas_policy_id, "cas.legacy_awaiting_pr_v1")
    t.eq(edge.cas_variant, "implementing_to_awaiting_pr")
    assert_valid_cas(edge)
  end,

  test_issue_legacy_pr_open_canonicalization_matches_production_marker = function()
    assert_observed_inventory_edge(4, observe_legacy_pr_open())
  end,

  test_issue_canonicalization_inventory_is_ordered_immutable_and_deterministic = function()
    local snapshot = copy_value(canonicalization_inventory)
    local authored = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)

    assert_canonicalization_shape(authored)
    assert_same_value(canonicalization_inventory, snapshot)
    t.eq(authored[1].id, "github-devloop/dependency_wait/canonicalization/legacy_ready_dependency_hold")
    t.eq(authored[2].id, "github-devloop/ready/canonicalization/legacy_ready_rederive")
    t.eq(authored[3].id, "github-devloop/awaiting-pr/canonicalization/implementing_merged_delegated_pr")
    t.eq(authored[4].id, "github-devloop/awaiting-pr/canonicalization/legacy_pr_open_delegation")

    local repeated = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)
    assert_canonicalization_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.cause_evidence ~= repeated[index].cause_evidence)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_canonicalization_edge_extractor_preserves_order_and_deep_copies = function()
    local inventory = {
      valid_canonicalization(),
      valid_canonicalization(
        "legacy_ready_dependency_hold",
        "ready",
        "dependency_wait"
      ),
    }
    inventory[2].provenance.row = "dependency_wait"
    inventory[2].provenance.field = "canonicalization_inventory.legacy_ready_dependency_hold"
    local snapshot = copy_value(inventory)

    local edges = restart_edges.extract_canonicalization_edges("owner", inventory)

    t.eq(#edges, 2)
    t.eq(edges[1].id, "owner/ready/canonicalization/legacy_ready_rederived")
    t.eq(edges[2].id, "owner/dependency_wait/canonicalization/legacy_ready_dependency_hold")
    assert_same_value(inventory, snapshot)
    for index, edge in ipairs(edges) do
      local expected = copy_value(inventory[index])
      expected.id = "owner/" .. expected.row_id .. "/canonicalization/" .. expected.semantic_variant
      assert_same_value(edge, expected)
      t.is_true(edge ~= inventory[index])
      t.is_true(edge.source ~= inventory[index].source)
      t.is_true(edge.cause_evidence ~= inventory[index].cause_evidence)
      t.is_true(edge.provenance ~= inventory[index].provenance)
    end
  end,

  test_canonicalization_edge_extractor_fails_closed_on_invalid_inventory = function()
    assert_extract_fails("", { valid_canonicalization() })
    assert_extract_fails("owner", nil)
    assert_extract_fails("owner", { "not-an-edge" })

    local edge = valid_canonicalization()
    edge.semantic_variant = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.semantic_variant = "qualified/legacy_ready_rederived"
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.row_id = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.kind = "entry"
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.source = nil
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.source.state = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.source.boundary = "unexpected-boundary"
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.target = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.cause_evidence = nil
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.cause_evidence.marker = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.cause_evidence.resolver = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.provenance = nil
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.provenance.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.provenance.row = ""
    assert_extract_fails("owner", { edge })

    edge = valid_canonicalization()
    edge.provenance.field = ""
    assert_extract_fails("owner", { edge })

    assert_extract_fails("owner", { valid_canonicalization(), valid_canonicalization() })
  end,
}
