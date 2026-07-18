-- workflow-security: security-review helpers.
--
-- This is the package's dedicated logic module. It is deliberately NOT named
-- `core` and holds NO engine/branching/idempotency/frontier/marker logic — all of
-- that lives exactly once in workflow.engine.* and is reached through the kernel
-- (see bindings.lua). This module owns only the security-domain shapes:
--   * the built-in `security-review` fkst.workflow.v1 template (records provider),
--   * the finding wire model (decode + validation + dedup key),
--   * the findings -> github-proxy issue-create request builder, and
--   * the codex analysis prompt frame for each generated step.
--
-- Every production error string carries a greppable `<class>: <class>:` prefix.
local strings = require("contract.strings")

local M = {}

-- Marker namespace + label are the adapter's identity. The namespace token is what
-- keeps this adapter's issue markers from colliding with a co-resident adapter's.
M.NAMESPACE = "fkst:workflow-security"
M.LABEL = "fkst-security"
M.WORKFLOW_ID = "security-review"
M.REVIEW_REQUEST_QUEUE = "security_review_request"
M.TICK_QUEUE = "workflow_security_tick"
M.MATERIALIZATION_TICK_QUEUE = "workflow_security_materialization_tick"
M.SECURITY_REVIEW_SEARCH = "label:fkst-security"

local github_proxy_limits = {
  repo = 200,
  title = 240,
  body = 12000,
  dedup_key = 512,
  source_ref_kind = 80,
  source_ref_ref = 200,
}

local finding_limits = {
  file = 240,
  area = 120,
  advisory = 200,
  summary = 1000,
  remediation = 1000,
}

-- Ordered severity vocabulary. A finding must name exactly one of these; the rank
-- orders the filed issues and is embedded in the dedup key so a re-run never
-- double-files the same finding.
local SEVERITY_RANK = {
  critical = 5,
  high = 4,
  medium = 3,
  low = 2,
  informational = 1,
}

local function bounded_field(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function marker_safe(value)
  return tostring(value):find('[<>"\r\n]') == nil
end

local function collapse_ws(value)
  return (tostring(value or ""):gsub("%s+", " "))
end

function M.security_label()
  return M.LABEL
end

function M.review_request_search_query()
  return M.SECURITY_REVIEW_SEARCH
end

-- Distinct from archaudit.validate_repo: same intent, an owner/name slug, but a
-- self-contained body (no forge dependency) so the dedup ratchet stays satisfied.
function M.repo_slug_ok(repo)
  if not strings.is_bounded_string(repo, github_proxy_limits.repo) then
    return false
  end
  return tostring(repo):match("^[%w._-]+/[%w._-]+$") ~= nil
end

function M.severity_rank(severity)
  return SEVERITY_RANK[tostring(severity or "")] or 0
end

-- A finding is the durable unit a review step emits. Steps 1-3 profile / match /
-- audit; the final step consolidates into an array of these.
function M.finding_ok(finding)
  if type(finding) ~= "table" then
    return false
  end
  if M.severity_rank(finding.severity) == 0 then
    return false
  end
  if not bounded_field(finding.area, finding_limits.area) then
    return false
  end
  if not bounded_field(finding.summary, finding_limits.summary) then
    return false
  end
  if not bounded_field(finding.remediation, finding_limits.remediation) then
    return false
  end
  if finding.file ~= nil and not bounded_field(finding.file, finding_limits.file) then
    return false
  end
  if finding.advisory ~= nil and not bounded_field(finding.advisory, finding_limits.advisory) then
    return false
  end
  return true
end

-- Decode a codex step's strict-JSON findings array. Rejects non-dense arrays and
-- any malformed member so a garbled analysis becomes a fatal step (not a filing).
function M.decode_findings(stdout)
  local raw = strings.trim(stdout or "")
  if raw == "" then
    return {}
  end
  if raw:sub(1, 1) ~= "[" or raw:sub(-1) ~= "]" then
    error("workflow-security: findings-not-array: analysis output is not a JSON array", 0)
  end
  local ok, decoded = pcall(json.decode, raw)
  if not ok or type(decoded) ~= "table" then
    error("workflow-security: findings-malformed-json: analysis output is malformed JSON", 0)
  end
  local highest = 0
  for key in pairs(decoded) do
    if type(key) ~= "number" or key < 1 or math.floor(key) ~= key then
      error("workflow-security: findings-not-array: analysis output is not a dense JSON array", 0)
    end
    if key > highest then
      highest = key
    end
  end
  if highest ~= #decoded then
    error("workflow-security: findings-not-array: analysis output has holes", 0)
  end
  local findings = {}
  for index, item in ipairs(decoded) do
    if not M.finding_ok(item) then
      error("workflow-security: finding-invalid-shape: index=" .. tostring(index), 0)
    end
    table.insert(findings, {
      severity = item.severity,
      area = item.area,
      file = item.file,
      advisory = item.advisory,
      summary = item.summary,
      remediation = item.remediation,
    })
  end
  return findings
end

-- Deterministic dedup key. Distinct construction from archaudit.dedup_key: seeded on
-- repo + area + advisory + severity + summary so re-filing the same finding is a
-- no-op at the github-proxy layer even when line numbers drift.
function M.finding_dedup_key(repo, finding)
  local seed = table.concat({
    tostring(repo),
    tostring(finding.severity),
    tostring(finding.area),
    tostring(finding.advisory or "none"),
    tostring(finding.file or "repo"),
    tostring(finding.summary),
  }, "\30")
  local readable = table.concat({
    "workflow-security",
    strings.sanitize_key(repo, 120),
    strings.sanitize_key(tostring(finding.severity), 24),
    strings.sanitize_key(tostring(finding.advisory or finding.area), 120),
    strings.decimal_checksum(seed),
  }, "/")
  return readable:sub(1, github_proxy_limits.dedup_key)
end

local function finding_title(finding)
  local location = finding.file and (" " .. collapse_ws(finding.file)) or ""
  return "Security[" .. tostring(finding.severity) .. "]: " .. collapse_ws(finding.area) .. location
end

local function finding_body(finding, dedup_key)
  local lines = {
    "Security review finding.",
    "",
    "Severity: " .. tostring(finding.severity),
    "Area: " .. tostring(finding.area),
  }
  if finding.file ~= nil then
    table.insert(lines, "Location: " .. tostring(finding.file))
  end
  if finding.advisory ~= nil then
    table.insert(lines, "Advisory: " .. tostring(finding.advisory))
  end
  table.insert(lines, "")
  table.insert(lines, "Summary:")
  table.insert(lines, tostring(finding.summary))
  table.insert(lines, "")
  table.insert(lines, "Remediation:")
  table.insert(lines, tostring(finding.remediation))
  table.insert(lines, "")
  table.insert(lines, "<!-- workflow-security-dedup: " .. tostring(dedup_key) .. " -->")
  return table.concat(lines, "\n")
end

local function require_bounded(ok, field)
  if not ok then
    error("workflow-security: invalid-issue-field: " .. tostring(field), 0)
  end
end

-- Build one github-proxy.github_issue_create_request for a finding. The label is
-- attached only when it exists in the repo (label_available), mirroring the
-- github-proxy issue-create contract; the dedup key makes filing idempotent.
function M.build_finding_issue_request(repo, finding, label_available)
  require_bounded(M.repo_slug_ok(repo), "repo")
  require_bounded(M.finding_ok(finding), "finding")
  local dedup_key = M.finding_dedup_key(repo, finding)
  local title = finding_title(finding)
  local body = finding_body(finding, dedup_key)
  local source_ref_ref = strings.sanitize_key(repo, 120)
    .. "#security/" .. strings.decimal_checksum(dedup_key)
  require_bounded(strings.is_bounded_string(title, github_proxy_limits.title), "title")
  require_bounded(strings.is_bounded_string(body, github_proxy_limits.body), "body")
  require_bounded(
    strings.is_bounded_string(dedup_key, github_proxy_limits.dedup_key) and marker_safe(dedup_key),
    "dedup_key"
  )
  require_bounded(strings.is_bounded_string(source_ref_ref, github_proxy_limits.source_ref_ref), "source_ref.ref")
  local labels = {}
  if label_available then
    labels = { M.LABEL }
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = tostring(repo),
    title = title,
    body = body,
    labels = labels,
    dedup_key = dedup_key,
    source_ref = {
      kind = "repo-site",
      ref = source_ref_ref,
    },
  }
end

-- Build the ordered issue-create requests for a whole review, highest severity
-- first and capped so one review cannot flood the tracker.
function M.build_finding_requests(repo, findings, label_available, max_requests)
  local cap = tonumber(max_requests) or 20
  local ordered = {}
  for _, finding in ipairs(findings or {}) do
    table.insert(ordered, finding)
  end
  table.sort(ordered, function(a, b)
    return M.severity_rank(a.severity) > M.severity_rank(b.severity)
  end)
  local requests = {}
  for _, finding in ipairs(ordered) do
    if #requests >= cap then
      break
    end
    table.insert(requests, M.build_finding_issue_request(repo, finding, label_available))
  end
  return requests
end

return M
