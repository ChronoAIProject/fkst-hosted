local canonicalization_inventory = require("core.restart.canonicalization_inventory")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")
local requests_review = require("devloop.requests.review")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local owner = "github-devloop-pr"
local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local proposal_id = "github-devloop/issue/owner/repo/42"
local branch = "devloop-owner-repo-42-01HY"
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
  ["github-devloop-pr/reviewing/canonicalization/fixing_head_renormalization"] = { participates = true, predecessor_state = "fixing" },
  ["github-devloop-pr/reviewing/canonicalization/pr_base_unmanaged_self_heal"] = { participates = false },
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

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-04T03:01:00Z",
  }
end

local function pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = pr_number,
    state = "OPEN",
    updated_at = "2026-06-04T01:02:04Z",
    dedup_key = "owner/repo#pr#7@2026-06-04T01:02:04Z",
    source_ref = entity_lib.pr_source_ref(repo, pr_number),
  }
end

local function assert_department_ok(result, site)
  if result.exit_code ~= 0 then
    error("restart canonicalization conformance: " .. site .. " failed: "
      .. tostring(result.error or result.stderr or "unknown department failure"))
  end
end

local function find_pr_comment_with(raises, needle)
  return h.find_raise(raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find(needle, 1, true) ~= nil
  end)
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

local function mock_issue_result_view(labels, comments)
  entity_read_mocks.mock_issue_read_forms(t, {
    labels = labels,
    comments = comments,
  })
  entity_read_mocks.mock_issue_view_selector(t, {
    labels = labels,
    comments = comments,
  }, "labels,comments")
  entity_read_mocks.mock_issue_view_selector(t, {}, "assignees,author")
end

local function mock_branch_config_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function run_observe_pr(name)
  mock_branch_config_env()
  t.mock_command(core.gh_issue_view_claim_cmd(repo, issue_number), {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = pr_event(),
  }, h.opts(name))
end

local function observe_fixing_head_renormalization()
  local event = h.fixing()
  local previous_version = event.version
  local version = core.next_fix_version(previous_version)
  local reviewed_head = "def456"
  local current_head = "feedface"
  local review_proposal = devloop_base.pr_review_proposal_id(repo, pr_number, previous_version, reviewed_head)
  local review_dedup = "consensus:" .. review_proposal .. "/review"
  local feedback = requests_review.build_review_result_comment_request(core, repo, issue_number,
    event.proposal_id, version, {
      proposal_id = review_proposal,
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
      dedup_key = review_dedup,
      source_ref = event.source_ref,
    }, event.source_ref).body
  local comments = {
    m_builders.pr_origin_marker(event.proposal_id, tostring(issue_number), branch, previous_version, "dev"),
    core.state_marker(event.proposal_id, "fixing", version),
    feedback,
  }
  local source = entity_lib.current_entity_state(comments, event.proposal_id)
  t.eq(source.state, "fixing")

  h.mock_bot_env()
  h.mock_pr_origin(comments, branch, current_head)
  mock_issue_result_view({ "fkst-dev:fixing" }, comments)
  t.mock_command("git fetch origin " .. branch, {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'FETCH_HEAD^{commit}'", {
    stdout = current_head .. "\n",
    stderr = "",
    exit_code = 0,
  })

  local result = run_observe_pr("restart-canonicalization-fixing-head-renormalization")
  assert_department_ok(result, "fixing-to-reviewing")
  local emitted = find_pr_comment_with(result.raises, "fkst:github-devloop:fix:v1")
  t.is_true(emitted ~= nil)
  local emitted_comments = { trusted_comment(emitted.payload.body) }
  t.eq(m_facts.has_fix_marker(emitted_comments, proposal_id,
    review_proposal, review_dedup, reviewed_head, current_head), true)
  local state = entity_lib.current_entity_state(emitted_comments, proposal_id)
  t.eq(state.state, "reviewing")
  t.eq(state.version, core.next_fix_version(source.version))
  return observed_edge(source.state, state.state, "fix:v1", "has_fix_marker")
end

local function mock_base_heal_env()
  for _ = 1, 2 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "integration",
    stderr = "",
    exit_code = 0,
  })
end

local function observe_base_unmanaged_self_heal()
  local impl_version = h.reviewing().version
  local blocked_version = requests_review.pr_base_unmanaged_blocked_version(impl_version)
  local comments = {
    m_builders.pr_origin_marker(proposal_id, tostring(issue_number), branch, impl_version, "integration"),
    core.state_marker(proposal_id, "blocked", blocked_version),
  }
  local source = entity_lib.current_entity_state(comments, proposal_id)
  t.eq(source.state, "blocked")
  t.eq(source.version, blocked_version)

  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    comments = comments,
    head = branch,
    head_sha = "def456",
    state = "OPEN",
    base_branch = "integration",
    labels = { "fkst-dev:blocked" },
  }, entity_read_mocks.pr_origin_selector)
  h.mock_issue_reviewing({ "fkst-dev:blocked" }, {
    core.state_marker(proposal_id, "blocked", blocked_version),
  }, {
    assignees = { "fkst-test-bot" },
  })

  mock_base_heal_env()
  local result = t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = pr_event(),
  }, h.opts("restart-canonicalization-pr-base-unmanaged-self-heal"))
  assert_department_ok(result, "blocked-to-reviewing")
  local emitted = find_pr_comment_with(result.raises, "fkst:github-devloop:state:v1")
  t.is_true(emitted ~= nil)
  local state = entity_lib.current_entity_state({ trusted_comment(emitted.payload.body) }, proposal_id)
  t.eq(state.state, "reviewing")
  t.eq(state.version, core.next_review_loop_version(impl_version))
  return observed_edge(source.state, state.state, "state:v1", "current_entity_state")
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
    edge_keys.pending_order = true
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { state = true })
    assert_exact_keys(edge.cause_evidence, { marker = true, resolver = true })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.kind, "canonicalization")
    t.eq(edge.source.boundary, nil)
    t.eq(edge.row_id, edge.target)
    t.eq(edge.provenance.owner, owner)
    t.eq(edge.provenance.row, edge.target)
    assert_same_value(edge.pending_order, pending_order_goldens[edge.id])
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function assert_observed_inventory_edge(index, observed)
  local snapshot = copy_value(canonicalization_inventory)
  local authored = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)
  assert_canonicalization_shape(authored)
  t.eq(#authored, 2)
  assert_symmetric_edge_sets({ observed }, { authored[index] })
  assert_same_value(canonicalization_inventory, snapshot)
end

return {
  test_pr_fixing_head_renormalization_matches_production_marker = function()
    assert_observed_inventory_edge(1, observe_fixing_head_renormalization())
  end,

  test_pr_base_unmanaged_self_heal_matches_production_marker = function()
    assert_observed_inventory_edge(2, observe_base_unmanaged_self_heal())
  end,

  test_pr_canonicalization_inventory_is_ordered_immutable_and_deterministic = function()
    local snapshot = copy_value(canonicalization_inventory)
    local authored = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)

    assert_canonicalization_shape(authored)
    assert_same_value(canonicalization_inventory, snapshot)
    t.eq(authored[1].id, "github-devloop-pr/reviewing/canonicalization/fixing_head_renormalization")
    t.eq(authored[2].id, "github-devloop-pr/reviewing/canonicalization/pr_base_unmanaged_self_heal")

    local repeated = restart_edges.extract_canonicalization_edges(owner, canonicalization_inventory)
    assert_canonicalization_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.cause_evidence ~= repeated[index].cause_evidence)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,
}
