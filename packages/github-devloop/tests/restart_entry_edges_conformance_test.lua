local base_ids = require("devloop.base_ids")
local entry_inventory = require("core.restart.entry_inventory")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local execution_start = require("devloop.execution_start")
local h = require("tests.devloop_helpers")
local restart_edges = require("devloop.restart_edges")

local core = h.core
local t = h.t

local owner = "github-devloop"
local structural_fields = {
  "id",
  "owner",
  "row_id",
  "kind",
  "source",
  "target",
  "provenance",
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

local function emitted_trusted_state(result, proposal_id)
  t.eq(result.exit_code, 0)
  local comment = h.find_raise(result.raises, "github-proxy.github_issue_comment_request")
  t.is_true(comment ~= nil)
  local authored = {
    body = comment.payload.body,
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:02:05Z",
  }
  local emitted = core.current_state({ authored }, proposal_id)
  t.eq(emitted.state, "thinking")

  authored.author_login = "untrusted-user"
  t.eq(core.current_state({ authored }, proposal_id).state, nil)
  return emitted.state
end

local function mock_empty_dependencies()
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function observe_unmanaged_issue_entry()
  local department = require("departments.observe_issue.main")
  local queue_name = consumed_queue(department.spec, "github-proxy.github_entity_changed")
  local payload = h.issue()
  local proposal_id = base_ids.proposal_id(payload.repo, payload.number)

  t.eq(core.current_state({}, proposal_id).state, nil)
  h.mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {})
  mock_empty_dependencies()
  h.mock_context_bundle(payload)
  local result = h.run_department("departments/observe_issue/main.lua", {
    queue = queue_name,
    payload = payload,
    ts = "2026-06-03T01:02:04Z",
  }, h.opts("restart-entry-observe-unmanaged"))

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = emitted_trusted_state(result, proposal_id),
  }
end

local function execution_request()
  return execution_start.build_execution_request_payload({
    proposal_id = "github-devloop/issue/owner/repo/42",
    dedup_key = "intake/github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    origin = {
      package = "github-devloop-intake-default",
      route = "default",
      decision = "enable",
    },
    service_class = "expedite",
  })
end

local function execute_request_entry()
  local department = require("departments.execute_start.main")
  local queue_name = consumed_queue(department.spec, "devloop_execute_request")
  local request = execution_request()

  h.mock_bot_env()
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = "owner/repo",
    number = 42,
    title = "Add retry backoff to failed widget sync",
    body = "Implement exponential backoff for widget sync retries.",
    updated_at = "2026-06-03T01:02:03Z",
    state = "OPEN",
    labels = {},
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", 1)
  h.mock_context_bundle(request)
  local result = t.run_department("departments/execute_start/main.lua", {
    queue = queue_name,
    payload = request,
    ts = "2026-06-03T01:02:04Z",
  }, h.opts("restart-entry-execute-request"))

  return {
    owner = owner,
    kind = "entry",
    boundary = durable_queue_name(queue_name),
    target = emitted_trusted_state(result, request.proposal_id),
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

local function assert_entry_shape(edges)
  local seen_ids = {}
  local edge_keys = key_set(structural_fields)
  for _, edge in ipairs(edges) do
    assert_exact_keys(edge, edge_keys)
    assert_exact_keys(edge.source, { boundary = true })
    assert_exact_keys(edge.provenance, { owner = true, row = true, field = true })
    t.eq(edge.owner, owner)
    t.eq(edge.row_id, "thinking")
    t.eq(edge.kind, "entry")
    t.eq(edge.source.state, nil)
    t.eq(edge.target, "thinking")
    t.eq(seen_ids[edge.id], nil)
    seen_ids[edge.id] = true
  end
end

local function valid_entry()
  return {
    id = "owner/thinking/entry/site",
    owner = "owner",
    row_id = "thinking",
    kind = "entry",
    source = { state = nil, boundary = "owner.queue" },
    target = "thinking",
    provenance = {
      owner = "owner",
      row = "thinking",
      field = "entry_inventory.site",
    },
  }
end

local function assert_extract_fails(selected_owner, inventory)
  local ok = pcall(function()
    restart_edges.extract_entry_edges(selected_owner, inventory)
  end)
  t.eq(ok, false)
end

return {
  test_issue_entry_inventory_matches_both_production_ingress_paths_symmetrically = function()
    local observed = {
      observe_unmanaged_issue_entry(),
      execute_request_entry(),
    }
    local snapshot = copy_value(entry_inventory)
    local authored = restart_edges.extract_entry_edges(owner, entry_inventory)

    assert_entry_shape(authored)
    assert_symmetric_edge_sets(observed, authored)
    assert_same_value(entry_inventory, snapshot)
    t.eq(authored[1].id, "github-devloop/thinking/entry/unmanaged_issue")
    t.eq(authored[2].id, "github-devloop/thinking/entry/execute_request")

    local repeated = restart_edges.extract_entry_edges(owner, entry_inventory)
    assert_entry_shape(repeated)
    for index, edge in ipairs(authored) do
      t.is_true(edge ~= repeated[index])
      t.is_true(edge.source ~= repeated[index].source)
      t.is_true(edge.provenance ~= repeated[index].provenance)
    end
  end,

  test_entry_edge_extractor_fails_closed_on_invalid_inventory = function()
    assert_extract_fails("", { valid_entry() })
    assert_extract_fails("owner", nil)
    assert_extract_fails("owner", { "not-an-edge" })

    local edge = valid_entry()
    edge.id = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.row_id = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.kind = "autonomous"
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.source.state = "unmanaged"
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.source.boundary = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.target = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.provenance.owner = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.provenance.owner = "other-owner"
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.provenance.row = ""
    assert_extract_fails("owner", { edge })

    edge = valid_entry()
    edge.provenance.field = ""
    assert_extract_fails("owner", { edge })

    assert_extract_fails("owner", { valid_entry(), valid_entry() })
  end,
}
