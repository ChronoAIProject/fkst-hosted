local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_issue_label_request" },
  stall_window = "30s",
}

local function normalize_labels(value)
  local labels = {}
  if type(value) ~= "table" then
    return labels
  end
  for _, label in ipairs(value) do
    if label ~= nil and tostring(label) ~= "" then
      table.insert(labels, tostring(label))
    end
  end
  return labels
end

local function describe_labels(add_labels, remove_labels)
  return "add=[" .. table.concat(add_labels, ",") .. "] remove=[" .. table.concat(remove_labels, ",") .. "]"
end

local function label_list(labels)
  return table.concat(labels or {}, ",")
end

local function log_outbound(payload, repo, add_labels, remove_labels, write_env)
  local mode = write_env == "1" and "real" or "dry-run"
  local fields = {
    "mode=" .. mode,
    "repo=" .. tostring(repo),
    "issue=" .. tostring(payload.issue_number),
    "add=" .. label_list(add_labels),
    "remove=" .. label_list(remove_labels),
    "dedup_key=" .. tostring(payload.dedup_key),
  }
  if mode == "dry-run" then
    table.insert(fields, "reason=FKST_GITHUB_WRITE!=1")
  end
  core.log_line("info", "github_issue_label", "OUTBOUND", fields)
end

local function log_skip(payload, repo, add_labels, remove_labels, reason)
  core.log_line("info", "github_issue_label", "SKIP", {
    "reason=" .. tostring(reason),
    "repo=" .. tostring(repo),
    "issue=" .. tostring(payload.issue_number),
    "add=" .. label_list(add_labels),
    "remove=" .. label_list(remove_labels),
    "dedup_key=" .. tostring(payload.dedup_key),
  })
end

function pipeline(event)
  local payload = event.payload or {}
  if payload.schema ~= "github-proxy.label.v1" then
    log.warn("github-proxy: unsupported label request schema")
    return
  end
  if payload.issue_number == nil or payload.dedup_key == nil then
    log.warn("github-proxy: label request missing issue_number or dedup_key")
    return
  end

  local repo = payload.repo or core.read_env("FKST_GITHUB_REPO")
  if repo == nil or repo == "" then
    log.warn("github-proxy: label request missing repo")
    return
  end

  local add_labels = normalize_labels(payload.add_labels)
  local remove_labels = normalize_labels(payload.remove_labels)
  if #add_labels == 0 and #remove_labels == 0 then
    log.warn("github-proxy: label request has no label changes")
    return
  end

  with_lock(core.issue_label_lock_key(repo, payload.issue_number), function()
    local write_env = core.read_env("FKST_GITHUB_WRITE")
    log_outbound(payload, repo, add_labels, remove_labels, write_env)
    if write_env ~= "1" then
      log.info("github-proxy dry-run: would set labels on "
        .. tostring(repo) .. "#" .. tostring(payload.issue_number) .. " "
        .. describe_labels(add_labels, remove_labels))
      return
    end
    if not core.verify_issue_claim_before_write(payload, repo, payload.issue_number, "github_issue_label") then
      return
    end

    local changed = core.apply_issue_labels(repo, payload.issue_number, add_labels, remove_labels)
    if changed ~= true then
      log_skip(payload, repo, add_labels, remove_labels, "no-effective-label-change")
    end
  end)
end

pipeline = core.wrap_pipeline_failure("github_issue_label", pipeline)

return M
