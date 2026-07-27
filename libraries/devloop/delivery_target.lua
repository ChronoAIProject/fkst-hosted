local base_ids = require("devloop.base_ids")
local config = require("devloop.config")
local forge_validators = require("devloop.forge_validators")
local strings = require("contract.strings")

local M = {}

local ENV_NAME = "FKST_DEVLOOP_DELIVERY_GRANTS"
local MAX_BYTES = 64 * 1024
local MAX_GRANTS = 128
local MAX_ROOT_BYTES = 1024
local GRANT_FIELDS = {
  lifecycle_repo = true,
  lifecycle_issue = true,
  implementation_repo = true,
  implementation_branch = true,
  implementation_root = true,
}

local function fail(message)
  error("github-devloop: delivery-grant-invalid: " .. tostring(message))
end

local function trim(value)
  return strings.trim(tostring(value or ""))
end

local function valid_repo(repo)
  local value = tostring(repo or "")
  local owner, name = value:match("^([%w_.%-]+)/([%w_.%-]+)$")
  return owner ~= nil
    and owner ~= ""
    and name ~= nil
    and name ~= ""
    and base_ids.issue_ref_round_trips(value, 1)
end

local function valid_root(root)
  local value = tostring(root or "")
  if value == "" or value == "/" or #value > MAX_ROOT_BYTES or value:sub(1, 1) ~= "/" then
    return false
  end
  if value:find("[\r\n%z]") ~= nil
    or value:find("//", 1, true) ~= nil
    or value:find("/./", 1, true) ~= nil
    or value:find("/../", 1, true) ~= nil
    or value:sub(-2) == "/."
    or value:sub(-3) == "/.." then
    return false
  end
  return value:sub(-1) ~= "/" and value:find("^[%w_./%-]+$") ~= nil
end

local function dense_array(value)
  local count = 0
  for key in pairs(value) do
    if type(key) ~= "number" or key < 1 or key ~= math.floor(key) then
      return nil
    end
    count = count + 1
  end
  for index = 1, count do
    if value[index] == nil then
      return nil
    end
  end
  return count
end

local function validate_grant(grant, index)
  if type(grant) ~= "table" then
    fail(ENV_NAME .. "[" .. tostring(index) .. "] must be an object")
  end
  for field in pairs(grant) do
    if not GRANT_FIELDS[field] then
      fail(ENV_NAME .. "[" .. tostring(index) .. "] contains unknown field " .. tostring(field))
    end
  end
  if not valid_repo(grant.lifecycle_repo) then
    fail(ENV_NAME .. "[" .. tostring(index) .. "].lifecycle_repo must be exactly owner/repo")
  end
  if not valid_repo(grant.implementation_repo) then
    fail(ENV_NAME .. "[" .. tostring(index) .. "].implementation_repo must be exactly owner/repo")
  end
  if type(grant.lifecycle_issue) ~= "number"
    or grant.lifecycle_issue < 1
    or grant.lifecycle_issue ~= math.floor(grant.lifecycle_issue) then
    fail(ENV_NAME .. "[" .. tostring(index) .. "].lifecycle_issue must be a positive integer")
  end
  if type(grant.implementation_branch) ~= "string"
    or not forge_validators.is_git_ref_safe(grant.implementation_branch) then
    fail(ENV_NAME .. "[" .. tostring(index) .. "].implementation_branch is unsafe")
  end
  if type(grant.implementation_root) ~= "string" or not valid_root(grant.implementation_root) then
    fail(ENV_NAME .. "[" .. tostring(index) .. "].implementation_root is unsafe")
  end
  if grant.lifecycle_repo:lower() == grant.implementation_repo:lower() then
    fail(ENV_NAME .. "[" .. tostring(index) .. "] must target a different repository")
  end
  return {
    lifecycle_repo = grant.lifecycle_repo,
    lifecycle_issue = grant.lifecycle_issue,
    implementation_repo = grant.implementation_repo,
    implementation_branch = grant.implementation_branch,
    implementation_root = grant.implementation_root,
  }
end

function M.parse(raw)
  local source = trim(raw)
  if source == "" then
    return {}
  end
  if #source > MAX_BYTES then
    fail(ENV_NAME .. " exceeds " .. tostring(MAX_BYTES) .. " bytes")
  end
  if source:sub(1, 1) ~= "[" or source:sub(-1) ~= "]" then
    fail(ENV_NAME .. " must be a JSON array")
  end
  local ok, decoded = pcall(json.decode, source)
  if not ok or type(decoded) ~= "table" then
    fail(ENV_NAME .. " must be valid JSON")
  end
  local count = dense_array(decoded)
  if count == nil then
    fail(ENV_NAME .. " must be a dense JSON array")
  end
  if count > MAX_GRANTS then
    fail(ENV_NAME .. " contains too many grants")
  end
  local grants = {}
  local identities = {}
  for index = 1, count do
    local grant = validate_grant(decoded[index], index)
    local identity = grant.lifecycle_repo:lower() .. "#" .. tostring(grant.lifecycle_issue)
    if identities[identity] then
      fail(ENV_NAME .. " contains duplicate lifecycle identity " .. identity)
    end
    identities[identity] = true
    table.insert(grants, grant)
  end
  return grants
end

local function normalized_origin_repo(origin)
  local value = trim(origin)
  local lower = value:lower()
  local marker_start = lower:find("github.com", 1, true)
  if marker_start == nil then
    return nil
  end
  local suffix = value:sub(marker_start + #"github.com")
  suffix = suffix:gsub("^[/:]+", ""):gsub("%.git$", ""):gsub("/+$", "")
  if valid_repo(suffix) then
    return suffix
  end
  return nil
end

local function require_git_fact(result, label)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop: delivery-checkout-mismatch: unable to verify " .. tostring(label))
  end
  return trim(result.stdout)
end

function M.verify_checkout(target, git)
  if type(target) ~= "table" or target.cross_repo ~= true then
    return target
  end
  if type(git) ~= "table" then
    error("github-devloop: delivery-checkout-mismatch: scoped git capability is required")
  end
  local top_level = require_git_fact(git.top_level(30), "checkout root")
  if top_level:gsub("/+$", "") ~= target.implementation_root then
    error("github-devloop: delivery-checkout-mismatch: checkout root differs from grant")
  end
  local origin_repo = normalized_origin_repo(require_git_fact(git.remote_url("origin", 30), "checkout origin"))
  if origin_repo == nil or origin_repo:lower() ~= target.implementation_repo:lower() then
    error("github-devloop: delivery-checkout-mismatch: checkout origin differs from grant")
  end
  local branch = require_git_fact(git.current_branch(30), "checkout branch")
  if branch ~= target.implementation_branch then
    error("github-devloop: delivery-checkout-mismatch: checkout branch differs from grant")
  end
  target.git = git
  return target
end

local function exact_grant(grants, lifecycle_repo, lifecycle_issue)
  local repo_folded = tostring(lifecycle_repo):lower()
  for _, grant in ipairs(grants) do
    if grant.lifecycle_repo:lower() == repo_folded
      and tostring(grant.lifecycle_issue) == tostring(lifecycle_issue) then
      return grant
    end
  end
  return nil
end

function M.resolve(lifecycle_repo, lifecycle_issue, opts)
  opts = opts or {}
  if not valid_repo(lifecycle_repo)
    or not base_ids.issue_ref_round_trips(lifecycle_repo, lifecycle_issue) then
    error("github-devloop: delivery-target-invalid: invalid lifecycle identity")
  end
  local raw = opts.raw
  if raw == nil then
    raw = config.read_env(ENV_NAME, opts.env_exec)
  end
  local grant = exact_grant(M.parse(raw), lifecycle_repo, lifecycle_issue)
  local asserted_repo = opts.implementation_repo
  local asserted_branch = opts.implementation_branch
  if grant == nil then
    if asserted_repo ~= nil and tostring(asserted_repo):lower() ~= tostring(lifecycle_repo):lower() then
      error("github-devloop: delivery-grant-missing: durable marker asserts ungranted cross-repository delivery")
    end
    return {
      lifecycle_repo = lifecycle_repo,
      lifecycle_issue = tonumber(lifecycle_issue),
      implementation_repo = lifecycle_repo,
      implementation_branch = asserted_branch or opts.default_branch,
      implementation_root = opts.default_root,
      cross_repo = false,
      git = opts.default_git,
    }
  end
  if asserted_repo ~= nil and tostring(asserted_repo):lower() ~= grant.implementation_repo:lower() then
    error("github-devloop: delivery-grant-mismatch: durable implementation repository differs from grant")
  end
  if asserted_branch ~= nil and tostring(asserted_branch) ~= grant.implementation_branch then
    error("github-devloop: delivery-grant-mismatch: durable implementation branch differs from grant")
  end
  local target = {
    lifecycle_repo = grant.lifecycle_repo,
    lifecycle_issue = grant.lifecycle_issue,
    implementation_repo = grant.implementation_repo,
    implementation_branch = grant.implementation_branch,
    implementation_root = grant.implementation_root,
    cross_repo = true,
  }
  if opts.verify ~= false then
    local git = opts.git
    if git == nil and type(opts.git_factory) == "function" then
      git = opts.git_factory(target.implementation_root)
    end
    M.verify_checkout(target, git)
  end
  return target
end

function M.implementation_lanes(lifecycle_repo, default_branch, opts)
  opts = opts or {}
  if not valid_repo(lifecycle_repo) then
    error("github-devloop: delivery-target-invalid: invalid lifecycle repository")
  end
  if type(default_branch) ~= "string" or not forge_validators.is_git_ref_safe(default_branch) then
    error("github-devloop: delivery-target-invalid: invalid default implementation branch")
  end
  local raw = opts.raw
  if raw == nil then
    raw = config.read_env(ENV_NAME, opts.env_exec)
  end
  local lanes = {
    {
      lifecycle_repo = lifecycle_repo,
      implementation_repo = lifecycle_repo,
      implementation_branch = default_branch,
      implementation_root = opts.default_root,
      cross_repo = false,
      git = opts.default_git,
    },
  }
  local seen = {
    [lifecycle_repo:lower() .. "\0" .. default_branch] = tostring(opts.default_root or ""),
  }
  for _, grant in ipairs(M.parse(raw)) do
    if grant.lifecycle_repo:lower() == lifecycle_repo:lower() then
      local key = grant.implementation_repo:lower() .. "\0" .. grant.implementation_branch
      local known_root = seen[key]
      if known_root ~= nil and known_root ~= grant.implementation_root then
        fail("implementation lane has conflicting checkout roots")
      end
      if known_root == nil then
        local target = {
          lifecycle_repo = lifecycle_repo,
          implementation_repo = grant.implementation_repo,
          implementation_branch = grant.implementation_branch,
          implementation_root = grant.implementation_root,
          cross_repo = true,
        }
        if opts.verify ~= false then
          local git = opts.git
          if git == nil and type(opts.git_factory) == "function" then
            git = opts.git_factory(target.implementation_root)
          end
          M.verify_checkout(target, git)
        end
        table.insert(lanes, target)
        seen[key] = grant.implementation_root
      end
    end
  end
  return lanes
end

function M.from_source_ref(lifecycle_repo, lifecycle_issue, source_ref, opts)
  local implementation_repo = select(1, require("devloop.base").parse_pr_source_ref(source_ref))
  local resolve_opts = {}
  for key, value in pairs(opts or {}) do resolve_opts[key] = value end
  resolve_opts.implementation_repo = implementation_repo or resolve_opts.implementation_repo
  return M.resolve(lifecycle_repo, lifecycle_issue, resolve_opts)
end

function M.for_entity(entity, source_ref, opts)
  if type(entity) ~= "table" then
    error("github-devloop: delivery-target-invalid: entity is required")
  end
  if entity.kind == "issue" then
    return M.from_source_ref(entity.repo, entity.issue_number, source_ref, opts)
  end
  if entity.kind ~= "pr" then
    error("github-devloop: delivery-target-invalid: unsupported entity kind")
  end
  local source_repo, source_pr = require("devloop.base").parse_pr_source_ref(source_ref)
  if source_repo ~= nil and (source_repo:lower() ~= tostring(entity.repo):lower()
      or tonumber(source_pr) ~= tonumber(entity.pr_number)) then
    error("github-devloop: delivery-target-mismatch: native PR source_ref differs from proposal")
  end
  return {
    lifecycle_repo = entity.repo,
    lifecycle_issue = nil,
    implementation_repo = entity.repo,
    implementation_branch = opts and opts.default_branch,
    implementation_root = opts and opts.default_root,
    cross_repo = false,
    git = opts and opts.default_git,
  }
end

function M.marker_repo(target)
  if type(target) == "table" and target.cross_repo == true then
    return target.implementation_repo
  end
  return nil
end

return M
