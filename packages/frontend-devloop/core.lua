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

local required_profile_duties = {
  "reusable platform package composition",
  "UI workflow trust-boundary declaration",
  "source-ref-only UI artifact handoff",
  "package-local conformance for the UI profile contract",
}

local necessity_alternatives = {
  {
    surface = "project-local scripts",
    owner = "host",
    existing_surfaces = {
      "package-manager scripts",
      ".fkst/compose/package-roots",
    },
    can_express = {
      "host-owned command execution",
      "host-local package root selection",
    },
    missing_profile_duties = {
      "reusable platform package composition",
      "UI workflow trust-boundary declaration",
      "package-local conformance for the UI profile contract",
    },
    insufficiency = "commands do not declare fkst package roots or trust boundaries",
    boundary_violation = "Host-local files would make each frontend host duplicate platform semantics that fkst-packages should validate once.",
    ownership_conflict = {
      actual_surface = ".fkst/compose/package-roots",
      current_authority = "host-local package root selection",
      must_not_own = {
        "UI workflow trust-boundary declaration",
        "source-ref-only UI artifact handoff",
      },
      reason = ".fkst/compose/package-roots is host input for selected roots, not the package-owned UI workflow profile authority.",
    },
  },
  {
    surface = "browser-qa",
    owner = "browser-qa",
    existing_surfaces = {
      "browser-qa",
    },
    can_express = {
      "browser execution",
      "visual validation",
    },
    missing_profile_duties = {
      "reusable platform package composition",
      "GitHub devloop lifecycle ownership",
    },
    insufficiency = "browser execution does not own devloop package composition",
    boundary_violation = "Putting package composition in browser-qa would couple browser execution to GitHub issue-to-PR lifecycle orchestration.",
    ownership_conflict = {
      actual_surface = "browser-qa",
      current_authority = "browser execution and visual validation",
      must_not_own = {
        "reusable platform package composition",
        "GitHub devloop lifecycle ownership",
      },
      reason = "browser-qa validates UI runtime behavior, not the package graph or issue-to-PR lifecycle.",
    },
  },
  {
    surface = "global-host profiles",
    owner = "host profile layer",
    existing_surfaces = {
      "global-host profiles",
    },
    can_express = {
      "generic host hydration",
      "workspace-root wiring",
    },
    missing_profile_duties = {
      "UI workflow trust-boundary declaration",
      "source-ref-only UI artifact handoff",
    },
    insufficiency = "generic host hydration does not own UI workflow artifact handoff",
    boundary_violation = "Putting UI artifact trust policy in the global host layer would couple generic host hydration to frontend workflow semantics.",
    ownership_conflict = {
      actual_surface = "global host profile environment",
      current_authority = "machine-local environment and workspace wiring",
      must_not_own = {
        "UI workflow trust-boundary declaration",
        "source-ref-only UI artifact handoff",
      },
      reason = "global host profiles intentionally exclude package roots and frontend workflow semantics.",
    },
  },
}

local function copy_list(list)
  local copied = {}
  for index, value in ipairs(list) do
    copied[index] = value
  end
  return copied
end

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local copied = {}
  for key, child in pairs(value) do
    copied[key] = copy_value(child)
  end
  return copied
end

local function copy_rows(rows)
  local copied = {}
  for index, row in ipairs(rows) do
    copied[index] = copy_value(row)
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

local function find_by_field(rows, field, expected)
  if type(rows) ~= "table" then
    return nil
  end
  for _, row in ipairs(rows) do
    if type(row) == "table" and row[field] == expected then
      return row
    end
  end
  return nil
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

local function require_necessity_alternative(alternatives, expected, ctx)
  local row = find_by_field(alternatives, "surface", expected.surface)
  if not row then
    error(ctx .. ": missing necessity proof for " .. expected.surface)
  end
  if row.owner ~= expected.owner then
    error(ctx .. ": invalid owner for " .. expected.surface)
  end
  if row.insufficiency ~= expected.insufficiency then
    error(ctx .. ": invalid insufficiency for " .. expected.surface)
  end
  for _, field in ipairs({ "existing_surfaces", "can_express", "missing_profile_duties" }) do
    local values = require_table(row, field, ctx .. ": " .. expected.surface)
    for _, value in ipairs(expected[field]) do
      require_list_contains(values, value, ctx .. ": " .. expected.surface .. " " .. field)
    end
  end
  if row.boundary_violation ~= expected.boundary_violation then
    error(ctx .. ": invalid boundary violation for " .. expected.surface)
  end
  local conflict = require_table(row, "ownership_conflict", ctx .. ": " .. expected.surface)
  local expected_conflict = expected.ownership_conflict
  if conflict.actual_surface ~= expected_conflict.actual_surface then
    error(ctx .. ": invalid ownership conflict surface for " .. expected.surface)
  end
  if conflict.current_authority ~= expected_conflict.current_authority then
    error(ctx .. ": invalid ownership conflict authority for " .. expected.surface)
  end
  local must_not_own = require_table(conflict, "must_not_own", ctx .. ": " .. expected.surface .. " ownership_conflict")
  for _, value in ipairs(expected_conflict.must_not_own) do
    require_list_contains(must_not_own, value, ctx .. ": " .. expected.surface .. " ownership_conflict")
  end
  if conflict.reason ~= expected_conflict.reason then
    error(ctx .. ": invalid ownership conflict reason for " .. expected.surface)
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
    necessity_proof = {
      schema = "frontend-devloop.necessity-proof.v1",
      required_profile_duties = copy_list(required_profile_duties),
      alternatives = copy_rows(necessity_alternatives),
      conclusion = "frontend-devloop owns the UI workflow profile contract",
    },
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
  local proof = require_table(profile, "necessity_proof", ctx)
  if proof.schema ~= "frontend-devloop.necessity-proof.v1" then
    error(ctx .. ": unsupported necessity proof schema")
  end
  local duties = require_table(proof, "required_profile_duties", ctx)
  for _, duty in ipairs(required_profile_duties) do
    require_list_contains(duties, duty, ctx .. ": necessity proof duties")
  end
  local alternatives = require_table(proof, "alternatives", ctx)
  for _, expected in ipairs(necessity_alternatives) do
    require_necessity_alternative(alternatives, expected, ctx)
  end
  if proof.conclusion ~= "frontend-devloop owns the UI workflow profile contract" then
    error(ctx .. ": invalid necessity proof conclusion")
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
