local M = {}

local error_facts = require("contract.error_facts")
local strings = require("contract.strings")
local gitref = require("forge.gitref")

local git_lookup_timeout_seconds = 30
local max_dedup_key_len = 512

local function trim(value)
  return strings.trim(value)
end

local function target_ref(target)
  return tostring(target.remote) .. "#" .. tostring(target.branch)
end

local function require_target_field(name, value, validator)
  local ok, result = pcall(validator, value)
  if not ok then
    error("git-branch-detector: watch-ref-invalid: invalid " .. name .. ": " .. tostring(result), 0)
  end
  return result
end

local function normalize_target(raw)
  local entry = trim(raw)
  local remote, branch = entry:match("^([^#]+)#(.+)$")
  if remote == nil or branch == nil then
    error("git-branch-detector: watch-ref-invalid: expected <remote>#<branch>", 0)
  end
  remote = require_target_field("remote", trim(remote), function(value)
    return gitref.require_safe_remote(value, "git-branch-detector")
  end)
  branch = require_target_field("branch", trim(branch), function(value)
    return gitref.require_safe_branch("branch", value, "git-branch-detector")
  end)
  return {
    remote = remote,
    branch = branch,
    ref = remote .. "#" .. branch,
  }
end

function M.parse_watch_refs(raw)
  local targets = {}
  for entry in tostring(raw or ""):gmatch("[^,%s]+") do
    table.insert(targets, normalize_target(entry))
  end
  return targets
end

local function first_nonempty_line(stdout)
  for line in tostring(stdout or ""):gmatch("[^\r\n]+") do
    if trim(line) ~= "" then
      return line
    end
  end
  return nil
end

function M.parse_ls_remote_branch_sha(stdout, target)
  local line = first_nonempty_line(stdout)
  if line == nil then
    return nil
  end

  local sha, ref = line:match("^(%x+)%s+([^%s]+)$")
  if sha == nil then
    error("git-branch-detector: git-ref-lookup-malformed: missing sha in ls-remote output", 0)
  end
  if ref ~= "refs/heads/" .. tostring(target.branch) then
    error("git-branch-detector: git-ref-lookup-malformed: unexpected ls-remote ref " .. tostring(ref), 0)
  end
  return gitref.require_safe_sha("remote branch sha", sha, "git-branch-detector")
end

function M.lookup_remote_branch_sha(git, target)
  if type(git) ~= "table" or type(git.ls_remote_branch) ~= "function" then
    error("git-branch-detector: git-port-unavailable: forge.git ls_remote_branch port is required", 0)
  end

  local result = git.ls_remote_branch(target.remote, target.branch, git_lookup_timeout_seconds)
  if type(result) ~= "table" then
    error("git-branch-detector: git-ref-lookup-failed: missing git result for " .. target_ref(target), 0)
  end
  if result.exit_code ~= 0 then
    error("git-branch-detector: git-ref-lookup-failed: " .. error_facts.one_line(result.stderr), 0)
  end
  return M.parse_ls_remote_branch_sha(result.stdout, target)
end

function M.lookup_error_class(message)
  local class = tostring(message or ""):match("git%-branch%-detector:%s*([%w%-]+):")
  if class ~= nil and class ~= "" then
    return class
  end
  return "git-ref-lookup-failed"
end

function M.lookup_failure_fact(dept, event, target, error_class, message)
  local why = tostring(message or "")
  local fields = error_facts.error_fact_fields(error_class, type(event) == "table" and event.queue or nil, dept, why, {
    source_ref = {
      kind = "git-ref",
      ref = target_ref(target),
    },
    terminal = true,
  })
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  return "git-branch-detector dept=" .. tostring(dept) .. " tag=FAIL_CLOSED " .. table.concat(fields, " ")
end

function M.observed_at(value)
  local numeric = tonumber(value)
  if numeric ~= nil then
    return os.date("!%Y-%m-%dT%H:%M:%SZ", math.floor(numeric))
  end
  local text = trim(value)
  if text == "" then
    error("git-branch-detector: observed-at-invalid: now value is required", 0)
  end
  return text
end

function M.changed_dedup_key(target, sha)
  local key = "git-ref/" .. target_ref(target) .. "#" .. tostring(sha)
  if not strings.is_path_safe_key(key, max_dedup_key_len) then
    error("git-branch-detector: dedup-key-invalid: invalid git_ref_changed dedup key", 0)
  end
  return key
end

function M.git_ref_changed_payload(target, sha, observed_at)
  local safe_sha = gitref.require_safe_sha("remote branch sha", sha, "git-branch-detector")
  return {
    schema = "git-branch-detector.ref-changed.v1",
    source_ref = {
      kind = "git-ref",
      ref = target_ref(target),
    },
    remote = tostring(target.remote),
    branch = tostring(target.branch),
    sha = safe_sha,
    observed_at = tostring(observed_at),
    dedup_key = M.changed_dedup_key(target, safe_sha),
  }
end

return M
