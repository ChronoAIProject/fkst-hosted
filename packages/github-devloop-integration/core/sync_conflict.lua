local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local S = {}
local strings = require("contract.strings")
local decimal_checksum = strings.decimal_checksum
local conflict_telemetry = require("devloop.conflict_telemetry")

function S.install(M)
local max_sync_conflict_attempts = 3

local function safe_branch_segment(branch)
  return strings.sanitize_key(tostring(branch or ""), false):gsub("/", "-")
end

local function conflict_fingerprint(conflict, unmerged_stdout)
  local paths = conflict_telemetry.conflict_file_paths_from_unmerged(M, unmerged_stdout)
  local material = {
    "repo=" .. tostring(conflict.repo or ""),
    "upstream=" .. tostring(conflict.upstream_branch or ""),
    "integration=" .. tostring(conflict.integration_branch or ""),
    "upstream_sha=" .. tostring(conflict.upstream_sha or ""),
    "integration_sha=" .. tostring(conflict.integration_sha or ""),
  }
  for _, path in ipairs(paths) do
    table.insert(material, "path=" .. path)
  end
  if #paths == 0 then
    local normalized = M._normalize_error_fact_text(unmerged_stdout or "")
    table.insert(material, "unmerged=" .. normalized)
  end
  return "sync-conflict-" .. decimal_checksum(table.concat(material, "\n"))
end

function M.max_sync_conflict_attempts()
  return max_sync_conflict_attempts
end

function M.sync_conflict_attempt_key(conflict, fingerprint)
  local readable = base_ids.safe_repo(conflict.repo)
    .. "/"
    .. safe_branch_segment(conflict.upstream_branch)
    .. "/"
    .. safe_branch_segment(conflict.integration_branch)
    .. "/"
    .. tostring(fingerprint or "")
  local suffix = decimal_checksum(readable)
  local key = "github-devloop/sync-conflict-attempt/"
    .. base_ids.safe_repo(conflict.repo)
    .. "/"
    .. safe_branch_segment(conflict.upstream_branch):sub(1, 40):gsub("%-+$", "")
    .. "/"
    .. safe_branch_segment(conflict.integration_branch):sub(1, 40):gsub("%-+$", "")
    .. "/"
    .. suffix
  if not strings.is_path_safe_key(key, M._max_dedup_len) then
    error("github-devloop: invalid sync conflict attempt key")
  end
  return key
end

function M.sync_conflict_fingerprint(conflict, unmerged_stdout)
  return conflict_fingerprint(conflict, unmerged_stdout)
end

function M.sync_conflict_attempt_count(conflict, fingerprint)
  local raw = cache_get(M.sync_conflict_attempt_key(conflict, fingerprint))
  local count = tonumber(raw)
  if count == nil or count < 0 or count ~= math.floor(count) then
    return 0
  end
  return count
end

function M.record_sync_conflict_attempt(conflict, fingerprint, attempt)
  local n = tonumber(attempt)
  if n == nil or n < 1 or n ~= math.floor(n) then
    error("github-devloop: invalid sync conflict attempt")
  end
  cache_set(M.sync_conflict_attempt_key(conflict, fingerprint), tostring(n))
  return n
end

function M.build_sync_conflict_escalation_request(conflict, fingerprint, attempt, reason, unmerged_stdout)
  local title = "Branch sync conflict requires manual resolution: "
    .. tostring(conflict.upstream_branch)
    .. " into "
    .. tostring(conflict.integration_branch)
  if #title > M._max_title_len then
    title = base_ids.truncate_utf8(title, M._max_title_len)
  end

  local paths = conflict_telemetry.conflict_file_paths_from_unmerged(M, unmerged_stdout)
  local path_lines = {}
  for _, path in ipairs(paths) do
    table.insert(path_lines, "- " .. path)
  end
  if #path_lines == 0 then
    table.insert(path_lines, "- no safe path list available")
  end

  local body = table.concat({
    "The autonomous branch sync conflict resolver exhausted its bounded retry budget.",
    "",
    "Reason: " .. devloop_base.neutralize_untrusted_comment_text(reason or "unresolved sync conflict"),
    "Attempt: " .. tostring(attempt),
    "Max attempts: " .. tostring(M.max_sync_conflict_attempts()),
    "Fingerprint: " .. tostring(fingerprint),
    "Repository: " .. tostring(conflict.repo),
    "Upstream branch: " .. tostring(conflict.upstream_branch),
    "Integration branch: " .. tostring(conflict.integration_branch),
    "Upstream head: " .. tostring(conflict.upstream_sha),
    "Integration parent: " .. tostring(conflict.integration_sha),
    "",
    "Unmerged paths:",
    table.concat(path_lines, "\n"),
    "",
    "Resolve the branch sync conflict manually or split the conflicting work so the rollup can make progress.",
  }, "\n")
  if #body > M._max_body_len then
    body = base_ids.truncate_utf8(body, M._max_body_len)
  end

  return {
    schema = "github-proxy.issue-create.v1",
    repo = conflict.repo,
    title = title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = base_ids.dedup_key({
      "sync-conflict-escalation",
      tostring(conflict.repo),
      tostring(conflict.upstream_branch),
      tostring(conflict.integration_branch),
      tostring(fingerprint),
    }),
    source_ref = base_ids.normalize_source_ref(conflict.source_ref),
  }
end
end

return S
