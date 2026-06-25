local M = {}

local strings = require("contract.strings")
local forge_strings = require("forge.strings")

local limits = {
  repo = 200,
  title = 240,
  body = 12000,
  dedup_key = 512,
  source_ref_kind = 80,
  source_ref_ref = 200,
}

function M.trim(value)
  return strings.trim(value)
end

function M.validate_repo(repo)
  if not strings.is_bounded_string(repo, limits.repo) then
    return false
  end
  if forge_strings.split_repo(repo) == nil then
    return false
  end
  return tostring(repo):find("^[%w._-]+/[%w._-]+$") ~= nil
end

local function decode_json_list(stdout, context)
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("integration-coverage-producer: malformed-json: " .. tostring(context), 0)
  end
  return decoded
end

function M.decode_checker_report(stdout)
  local decoded = decode_json_list(stdout, "coverage checker")
  local entries = {}
  for index, entry in ipairs(decoded) do
    if type(entry) ~= "table" then
      error("integration-coverage-producer: malformed-checker-entry: entry " .. tostring(index), 0)
    end
    table.insert(entries, entry)
  end
  return entries
end

function M.uncovered_allowlisted_edges(report)
  local edges = {}
  for _, entry in ipairs(report or {}) do
    if entry.status == "uncovered-allowlisted" then
      if type(entry.edge_id) ~= "string" or entry.edge_id == "" then
        error("integration-coverage-producer: malformed-edge: missing edge_id", 0)
      end
      table.insert(edges, entry)
    end
  end
  return edges
end

function M.decode_issue_search(stdout, context)
  return decode_json_list(stdout, context or "issue search")
end

function M.has_open_issue_with_marker(issues, marker)
  for _, issue in ipairs(issues or {}) do
    if type(issue) == "table"
      and tostring(issue.state or ""):upper() ~= "CLOSED"
      and tostring(issue.body or ""):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.decode_open_issue_list(stdout)
  return decode_json_list(stdout, "open issue list")
end

function M.open_issue_fields()
  return "number,title,state,labels"
end

function M.open_issue_limit()
  return 100
end

function M.open_issue_state()
  return "open"
end

function M.has_devloop_label(issue)
  if type(issue) ~= "table" then
    return false
  end
  for _, label in ipairs(issue.labels or {}) do
    local name = type(label) == "table" and label.name or label
    if tostring(name or ""):sub(1, #"fkst-dev:") == "fkst-dev:" then
      return true
    end
  end
  return false
end

function M.devloop_issue_count(issues)
  local count = 0
  for _, issue in ipairs(issues or {}) do
    if type(issue) == "table"
      and tostring(issue.state or ""):upper() ~= "CLOSED"
      and M.has_devloop_label(issue) then
      count = count + 1
    end
  end
  return count
end

function M.coverage_marker(edge_id)
  return "coverage-edge-id: " .. tostring(edge_id)
end

local function marker_safe(value)
  return tostring(value):find('[<>"\r\n]') == nil
end

local function safe_key(value, limit)
  return strings.sanitize_key(tostring(value or ""):gsub("%s+", "-"), limit)
end

function M.issue_search_query(edge_id)
  return M.coverage_marker(edge_id)
end

local function assert_field(ok, field)
  if not ok then
    error("integration-coverage-producer: invalid-issue-create-field: " .. tostring(field), 0)
  end
end

local function body(edge)
  return table.concat({
    M.coverage_marker(edge.edge_id),
    "",
    "Edge:",
    "- Queue: " .. tostring(edge.queue),
    "- Producer: " .. tostring(edge.producer_pkg) .. "." .. tostring(edge.producer_dept),
    "- Consumer: " .. tostring(edge.consumer_pkg) .. "." .. tostring(edge.consumer_dept),
    "- Owner scope: " .. tostring(edge.owner_scope),
    "",
    "Deliverable:",
    "Add one Lua package run_graph test for this cross-package edge using `graph.assert_covers`.",
    "",
    "Acceptance:",
    "- The new test drives the real producer-to-consumer path and covers this exact edge.",
    "- Remove the allowlist line for this edge from `migration/integration-edge-coverage.allowlist`.",
    "- `python3 scripts/check_repo.py` passes.",
    "- `python3 scripts/check_repo_integration_coverage.py --json` no longer reports this edge as `uncovered-allowlisted`.",
  }, "\n")
end

function M.issue_create_request(repo, edge)
  assert_field(M.validate_repo(repo), "repo")
  assert_field(type(edge) == "table", "edge")
  assert_field(strings.is_bounded_string(edge.edge_id, 240), "edge_id")
  local title = "test: run_graph coverage for " .. tostring(edge.edge_id)
  local body_text = body(edge)
  local dedup_key = table.concat({
    "integration-coverage",
    safe_key(repo, 120),
    safe_key(edge.edge_id, 260),
    strings.decimal_checksum(tostring(repo) .. "|" .. tostring(edge.edge_id)),
  }, "/")
  dedup_key = dedup_key:sub(1, limits.dedup_key)
  local source_ref_ref = safe_key(repo, 120) .. "#integration-coverage/" .. strings.decimal_checksum(edge.edge_id)
  assert_field(strings.is_bounded_string(title, limits.title), "title")
  assert_field(strings.is_bounded_string(body_text, limits.body), "body")
  assert_field(strings.is_bounded_string(dedup_key, limits.dedup_key) and marker_safe(dedup_key), "dedup_key")
  assert_field(strings.is_bounded_string("repo-site", limits.source_ref_kind), "source_ref.kind")
  assert_field(strings.is_bounded_string(source_ref_ref, limits.source_ref_ref), "source_ref.ref")
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = title,
    body = body_text,
    labels = {},
    dedup_key = dedup_key,
    source_ref = {
      kind = "repo-site",
      ref = source_ref_ref,
    },
  }
end

return M
