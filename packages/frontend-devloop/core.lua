local M = {}

local required_platform_packages = {
  "github-proxy",
  "consensus",
  "github-devloop-intake",
  "github-devloop-intake-default",
  "github-devloop-decompose",
  "github-devloop",
  "github-devloop-pr",
  "github-devloop-ops",
  "github-devloop-integration",
  "frontend-devloop",
}

local required_commands = {
  "install",
  "lint",
  "test",
  "build",
}

local trust_boundaries = {
  "issue-content-untrusted",
  "browser-results-untrusted",
  "host-scripts-owned-by-host",
}

local function copy_list(list)
  local copied = {}
  for index, value in ipairs(list) do
    copied[index] = value
  end
  return copied
end

local function has_item(list, expected)
  if type(list) ~= "table" then
    return false
  end
  for _, value in ipairs(list) do
    if value == expected then
      return true
    end
  end
  return false
end

local function require_string(row, field, ctx)
  local value = type(row) == "table" and row[field] or nil
  if type(value) ~= "string" or value == "" then
    error(ctx .. ": missing " .. field)
  end
  return value
end

local function require_table(row, field, ctx)
  local value = type(row) == "table" and row[field] or nil
  if type(value) ~= "table" then
    error(ctx .. ": missing " .. field)
  end
  return value
end

local function require_list_contains(list, value, ctx)
  if not has_item(list, value) then
    error(ctx .. ": missing " .. value)
  end
end

function M.platform_packages()
  return copy_list(required_platform_packages)
end

function M.default_profile()
  return {
    schema = "frontend-devloop.profile.v1",
    name = "frontend-devloop",
    owns = "host-ui-application-workflow-profile",
    issue_lifecycle_owner = "github-devloop",
    browser_qa_owner = "browser-qa",
    platform_packages = M.platform_packages(),
    host_capabilities = {
      required_commands = copy_list(required_commands),
      command_contract = "project-local package-manager scripts or host-owned command adapters",
      artifact_contract = "host worktree and generated UI artifacts stay source_ref-addressed",
    },
    handoff = {
      schema = "frontend-devloop.handoff.v1",
      payload_policy = "source-ref-only",
      ui_artifact_source_ref = {
        kind = "host-worktree",
        ref = "host://ui-artifacts",
      },
      trust_boundaries = copy_list(trust_boundaries),
    },
    non_scope = {
      "browser automation execution",
      "GitHub issue lifecycle state machine",
      "host package-manager implementation",
    },
  }
end

function M.validate_profile(profile)
  local ctx = "frontend-devloop: invalid-profile"
  if type(profile) ~= "table" then
    error(ctx .. ": profile must be a table")
  end
  if profile.schema ~= "frontend-devloop.profile.v1" then
    error(ctx .. ": unsupported schema")
  end
  if profile.name ~= "frontend-devloop" then
    error(ctx .. ": unsupported name")
  end
  if profile.owns ~= "host-ui-application-workflow-profile" then
    error(ctx .. ": invalid ownership")
  end
  if profile.issue_lifecycle_owner ~= "github-devloop" then
    error(ctx .. ": issue lifecycle owner must be github-devloop")
  end
  if profile.browser_qa_owner ~= "browser-qa" then
    error(ctx .. ": browser QA owner must be browser-qa")
  end
  local packages = require_table(profile, "platform_packages", ctx)
  for _, package_name in ipairs(required_platform_packages) do
    require_list_contains(packages, package_name, ctx)
  end
  local capabilities = require_table(profile, "host_capabilities", ctx)
  local commands = require_table(capabilities, "required_commands", ctx)
  for _, command in ipairs(required_commands) do
    require_list_contains(commands, command, ctx)
  end
  require_string(capabilities, "command_contract", ctx)
  require_string(capabilities, "artifact_contract", ctx)
  local handoff = require_table(profile, "handoff", ctx)
  if handoff.schema ~= "frontend-devloop.handoff.v1" then
    error(ctx .. ": unsupported handoff schema")
  end
  if handoff.payload_policy ~= "source-ref-only" then
    error(ctx .. ": UI artifacts must be source-ref-only")
  end
  local source_ref = require_table(handoff, "ui_artifact_source_ref", ctx)
  if source_ref.kind ~= "host-worktree" then
    error(ctx .. ": UI artifact source_ref must be host-worktree")
  end
  require_string(source_ref, "ref", ctx)
  local boundaries = require_table(handoff, "trust_boundaries", ctx)
  for _, boundary in ipairs(trust_boundaries) do
    require_list_contains(boundaries, boundary, ctx)
  end
  return profile
end

function M.host_package_roots_contract()
  return {
    schema = "frontend-devloop.host-package-roots.v1",
    profile = "frontend-devloop",
    project_root_owner = "host",
    platform_source = "fkst-packages-platform",
    compose_file = ".fkst/compose/package-roots",
    required_entries = {
      "fkst-packages:packages/github-proxy",
      "fkst-packages:packages/consensus",
      "fkst-packages:packages/github-devloop-intake",
      "fkst-packages:packages/github-devloop-intake-default",
      "fkst-packages:packages/github-devloop-decompose",
      "fkst-packages:packages/github-devloop",
      "fkst-packages:packages/github-devloop-pr",
      "fkst-packages:packages/github-devloop-ops",
      "fkst-packages:packages/github-devloop-integration",
      "fkst-packages:packages/frontend-devloop",
    },
  }
end

return M
