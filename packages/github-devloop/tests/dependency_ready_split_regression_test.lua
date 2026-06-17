local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
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

local function issue_view_json(labels, comments, state)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', encode_json_string(label)))
  end
  return string.format(
    '{"title":"Implement dependency split","state":"%s","labels":[%s],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}]}\n',
    encode_json_string(state or "OPEN"),
    table.concat(rendered_labels, ","),
    issue_comments_json(comments)
  )
end

local function blocked_by_json(nodes)
  local rendered = {}
  local input = nodes or {}
  for _, node in ipairs(input) do
    local state_reason = node.state_reason or node.stateReason or ""
    table.insert(rendered, string.format(
      '{"number":%s,"state":"%s","stateReason":"%s","repository":{"nameWithOwner":"%s"}}',
      tostring(node.number),
      encode_json_string(node.state or "OPEN"),
      encode_json_string(state_reason),
      encode_json_string(node.repo or repo)
    ))
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":'
    .. tostring(#input)
    .. ',"pageInfo":{"hasNextPage":false},"nodes":['
    .. table.concat(rendered, ",")
    .. ']}}}}}\n'
end

local function mock_blocked_by(issue_number, nodes)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = blocked_by_json(nodes),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocked_by_failure(issue_number)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = "",
    stderr = "graphql failed",
    exit_code = 1,
  })
end

local function mock_blocker_issue(issue_number, state_name)
  local comments = {}
  if state_name ~= nil then
    table.insert(comments, core.state_marker(core.proposal_id(repo, issue_number), state_name, "v-" .. tostring(issue_number)))
  end
  t.mock_command(core.gh_issue_view_observe_cmd(repo, issue_number), {
    stdout = '{"state":"OPEN","comments":[' .. issue_comments_json(comments) .. ']}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_observe_issue(labels, comments)
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 42,
    labels = labels,
    comments = comments,
    times = 1,
  })
  t.mock_command(core.gh_issue_view_entity_cmd(repo, 42), {
    stdout = issue_view_json(labels, comments),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_implement_issue(labels, comments)
  t.mock_command(core.gh_issue_view_implement_cmd(repo, 42), {
    stdout = issue_view_json(labels, comments),
    stderr = "",
    exit_code = 0,
  })
end

local function reached()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal_id,
    decision = "approve",
    body = "Approved.",
    dedup_key = version,
    source_ref = source_ref(),
  }
end

local function ready_at(inner_version)
  return core.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = inner_version,
    source_ref = source_ref(),
  })
end

local function run_observe()
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = h.issue(),
  }, h.opts("ready-split-regression-observe"))
end

local function run_implement(payload)
  return t.run_department("departments/implement/main.lua", {
    queue = "devloop_ready",
    payload = payload,
  }, h.opts("ready-split-regression-implement"))
end

local function find_raise(raises, queue, predicate)
  for _, item in ipairs(raises or {}) do
    if item.queue == queue and (predicate == nil or predicate(item.payload)) then
      return item
    end
  end
  return nil
end

local function count_queue(raises, queue)
  local count = 0
  for _, item in ipairs(raises or {}) do
    if item.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function marker_body(raises, needle)
  local raise = find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return type(payload.body) == "string" and payload.body:find(needle, 1, true) ~= nil
  end)
  return raise and raise.payload.body or nil
end

return {
  test_implement_backstop_split_generation_uses_inner_ready_version = function()
    local split_version = core.ready_split_version(version)
    local ready = ready_at(split_version)
    mock_blocked_by(42, { { number = 55 } })
    mock_blocked_by(55, {})
    mock_implement_issue({ "fkst-dev:ready" }, {
      core.state_marker(proposal_id, "ready", split_version),
    })

    local result = run_implement(ready)
    t.eq(result.exit_code, 0)
    t.eq(count_queue(result.raises, "github-proxy.github_issue_comment_request"), 1)
    t.eq(count_queue(result.raises, "github-proxy.github_issue_label_request"), 1)
    local body = marker_body(result.raises, "ready-split-canonicalized:v1")
    local inner_version = core.ready_payload_inner_version(ready.dedup_key)
    local next_split_version = core.ready_split_version(inner_version)
    t.is_true(body ~= nil)
    t.is_true(body:find('from_version="' .. inner_version .. '"', 1, true) ~= nil)
    t.is_true(body:find('to_version="' .. next_split_version .. '"', 1, true) ~= nil)
    t.is_true(body:find('to_version="ready/', 1, true) == nil)
    t.is_true(body:find('state="dependency_wait"', 1, true) ~= nil)
  end,

  test_legacy_ready_unresolvable_hold_canonicalizes_to_dependency_wait = function()
    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "ready", version),
        "github-devloop dependency hold: unresolvable\n\nReason: gh-failed\n\n"
          .. core.dependency_unresolvable_marker(proposal_id, version, { 42 }),
      }
    )
    mock_blocked_by_failure(42)

    local result = run_observe()
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    local split_version = core.ready_split_version(version)
    local body = marker_body(result.raises, "ready-split-canonicalized:v1")
    t.is_true(body ~= nil)
    t.is_true(body:find('derived_state="dependency_wait"', 1, true) ~= nil)
    t.is_true(body:find('state="dependency_wait"', 1, true) ~= nil)
    t.is_true(body:find('version="' .. split_version .. '"', 1, true) ~= nil)
    t.is_true(body:find("fkst:github-devloop:dependency-wait:v1", 1, true) ~= nil)
  end,

  test_consensus_result_reraises_partial_dependency_wait_effects = function()
    local current = reached()
    h.mock_issue_result({ "fkst-dev:ready" }, {
      core.state_marker(current.proposal_id, "dependency_wait", current.dedup_key),
      core.result_marker(current.proposal_id, current.decision, current.dedup_key),
    })
    mock_blocked_by(42, { { number = 51 } })
    mock_blocked_by(51, {})
    mock_blocker_issue(51, "ready")

    local result = h.run_result(current, h.opts("ready-split-regression-result"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    t.is_true(marker_body(result.raises, "fkst:github-devloop:dependency-wait:v1") ~= nil)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return h.has_value(payload.add_labels, "fkst-dev:blocked-on-dependency")
    end)
    t.is_true(label ~= nil)
  end,
}
