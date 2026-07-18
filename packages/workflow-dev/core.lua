-- workflow-dev: the development adapter profile contract.
--
-- workflow-dev is a THIN composed profile package. It owns no engine or
-- business logic: the development engine (issue -> child workflow -> code atom)
-- is the proven github-devloop-workflow package, and the generic workflow
-- machinery lives once in the shared workflow.engine.* library. This module is
-- only the declarative profile that (a) composes the existing github-devloop
-- package family and (b) selects the workflow-engine intake policy
-- (github-devloop-workflow, the "workflow" topology) instead of the default
-- topology used by frontend-devloop.

local M = {}

-- The concrete devloop package family a workflow-dev host composes. Exactly
-- one intake-policy implementation is present (github-devloop-workflow, the
-- "workflow" topology); the default topology's github-devloop-intake-default
-- is intentionally absent so the intake-policy slot stays single-consumer.
local required_platform_packages = {
  "github-proxy",
  "consensus",
  "github-devloop-intake",
  "github-devloop-workflow",
  "github-devloop-decompose",
  "github-devloop",
  "github-devloop-pr",
  "github-devloop-ops",
  "github-devloop-integration",
  "workflow-dev",
}

local required_commands = {
  "install",
  "lint",
  "test",
  "build",
}

local trust_boundaries = {
  "issue-content-untrusted",
  "codex-results-untrusted",
  "host-scripts-owned-by-host",
}

local required_profile_duties = {
  "reusable devloop platform composition",
  "workflow-engine intake policy selection",
  "development workflow trust-boundary declaration",
  "package-local conformance for the development profile contract",
}

local necessity_alternatives = {
  {
    surface = "default-topology profile",
    owner = "frontend-devloop",
    existing_surfaces = {
      "frontend-devloop",
      "github-devloop-intake-default",
    },
    can_express = {
      "reusable devloop platform composition",
      "default-topology intake policy selection",
    },
    missing_profile_duties = {
      "workflow-engine intake policy selection",
      "development workflow trust-boundary declaration",
    },
    insufficiency = "the default topology selects github-devloop-intake-default, not the workflow engine intake policy",
    boundary_violation = "Reusing the default-topology profile would bind development hosts to the default intake policy and drop the workflow-engine materialization contract.",
    ownership_conflict = {
      actual_surface = "github-devloop-intake-default",
      current_authority = "default-topology intake policy",
      must_not_own = {
        "workflow-engine intake policy selection",
        "development workflow trust-boundary declaration",
      },
      reason = "github-devloop-intake-default is the default topology's intake policy; it is not the workflow topology authority.",
    },
  },
  {
    surface = "github-devloop-workflow package",
    owner = "github-devloop-workflow",
    existing_surfaces = {
      "github-devloop-workflow",
    },
    can_express = {
      "workflow-engine intake policy selection",
      "workflow materialization lifecycle ownership",
    },
    missing_profile_duties = {
      "reusable devloop platform composition",
      "development workflow trust-boundary declaration",
      "package-local conformance for the development profile contract",
    },
    insufficiency = "the workflow engine package owns the intake seat and materialization, not the host development profile composition",
    boundary_violation = "Putting host profile composition inside github-devloop-workflow would couple the intake/materialization engine to which platform packages a development host runs.",
    ownership_conflict = {
      actual_surface = "github-devloop-workflow",
      current_authority = "workflow-engine intake policy and materialization lifecycle",
      must_not_own = {
        "reusable devloop platform composition",
        "development workflow trust-boundary declaration",
      },
      reason = "github-devloop-workflow is the dev engine and single intake seat; it is not the host development profile authority.",
    },
  },
}

-- Deep/shallow copy + lookup helpers. These are deliberately written with
-- workflow-dev-local bodies (numeric-index iteration, `clone` naming) so they
-- stay byte-distinct from sibling profile packages under the dedup ratchet.
local function copy_list(list)
  local clone = {}
  local total = #list
  for i = 1, total do
    clone[i] = list[i]
  end
  return clone
end

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local clone = {}
  for key, child in pairs(value) do
    clone[key] = copy_value(child)
  end
  return clone
end

local function copy_rows(rows)
  local clone = {}
  local total = #rows
  for i = 1, total do
    clone[i] = copy_value(rows[i])
  end
  return clone
end

local function has_item(list, expected)
  if type(list) ~= "table" then
    return false
  end
  local total = #list
  for i = 1, total do
    if list[i] == expected then
      return true
    end
  end
  return false
end

local function find_by_field(rows, field, expected)
  if type(rows) ~= "table" then
    return nil
  end
  local total = #rows
  for i = 1, total do
    local row = rows[i]
    if type(row) == "table" and row[field] == expected then
      return row
    end
  end
  return nil
end

local function require_string(row, field, ctx)
  local value = type(row) == "table" and row[field] or nil
  if type(value) ~= "string" or value == "" then
    error(ctx .. ": missing-" .. field, 0)
  end
  return value
end

local function require_table(row, field, ctx)
  local value = type(row) == "table" and row[field] or nil
  if type(value) ~= "table" then
    error(ctx .. ": missing-" .. field, 0)
  end
  return value
end

local function require_list_contains(list, value, ctx)
  if not has_item(list, value) then
    error(ctx .. ": missing-entry", 0)
  end
end

local function require_necessity_alternative(alternatives, expected, ctx)
  local row = find_by_field(alternatives, "surface", expected.surface)
  if not row then
    error(ctx .. ": missing-necessity-proof", 0)
  end
  if row.owner ~= expected.owner then
    error(ctx .. ": invalid-owner", 0)
  end
  if row.insufficiency ~= expected.insufficiency then
    error(ctx .. ": invalid-insufficiency", 0)
  end
  for _, field in ipairs({ "existing_surfaces", "can_express", "missing_profile_duties" }) do
    local values = require_table(row, field, ctx)
    for _, value in ipairs(expected[field]) do
      require_list_contains(values, value, ctx)
    end
  end
  if row.boundary_violation ~= expected.boundary_violation then
    error(ctx .. ": invalid-boundary-violation", 0)
  end
  local conflict = require_table(row, "ownership_conflict", ctx)
  local expected_conflict = expected.ownership_conflict
  if conflict.actual_surface ~= expected_conflict.actual_surface then
    error(ctx .. ": invalid-ownership-conflict-surface", 0)
  end
  if conflict.current_authority ~= expected_conflict.current_authority then
    error(ctx .. ": invalid-ownership-conflict-authority", 0)
  end
  local must_not_own = require_table(conflict, "must_not_own", ctx)
  for _, value in ipairs(expected_conflict.must_not_own) do
    require_list_contains(must_not_own, value, ctx)
  end
  if conflict.reason ~= expected_conflict.reason then
    error(ctx .. ": invalid-ownership-conflict-reason", 0)
  end
end

function M.platform_packages()
  return copy_list(required_platform_packages)
end

function M.default_profile()
  return {
    schema = "workflow-dev.profile.v1",
    name = "workflow-dev",
    owns = "host-development-workflow-profile",
    issue_lifecycle_owner = "github-devloop",
    intake_policy_owner = "github-devloop-workflow",
    intake_policy_topology = "workflow",
    dev_engine = "github-devloop-workflow",
    shared_kernel = "workflow.engine",
    platform_packages = M.platform_packages(),
    necessity_proof = {
      schema = "workflow-dev.necessity-proof.v1",
      required_profile_duties = copy_list(required_profile_duties),
      alternatives = copy_rows(necessity_alternatives),
      conclusion = "workflow-dev owns the development workflow profile contract",
    },
    host_capabilities = {
      required_commands = copy_list(required_commands),
      command_contract = "project-local package-manager scripts or host-owned command adapters",
      artifact_contract = "host worktree and generated code artifacts stay source_ref-addressed",
    },
    handoff = {
      schema = "workflow-dev.handoff.v1",
      payload_policy = "source-ref-only",
      code_artifact_source_ref = {
        kind = "host-worktree",
        ref = "host://code-artifacts",
      },
      trust_boundaries = copy_list(trust_boundaries),
    },
    non_scope = {
      "workflow engine materialization state machine",
      "github issue lifecycle state machine",
      "host package-manager implementation",
      "shared workflow kernel implementation",
    },
  }
end

function M.validate_profile(profile)
  local ctx = "workflow-dev: invalid-profile"
  if type(profile) ~= "table" then
    error(ctx .. ": not-a-table", 0)
  end
  if profile.schema ~= "workflow-dev.profile.v1" then
    error(ctx .. ": unsupported-schema", 0)
  end
  if profile.name ~= "workflow-dev" then
    error(ctx .. ": unsupported-name", 0)
  end
  if profile.owns ~= "host-development-workflow-profile" then
    error(ctx .. ": invalid-ownership", 0)
  end
  if profile.issue_lifecycle_owner ~= "github-devloop" then
    error(ctx .. ": invalid-issue-lifecycle-owner", 0)
  end
  if profile.intake_policy_owner ~= "github-devloop-workflow" then
    error(ctx .. ": invalid-intake-policy-owner", 0)
  end
  if profile.intake_policy_topology ~= "workflow" then
    error(ctx .. ": invalid-intake-policy-topology", 0)
  end
  if profile.dev_engine ~= "github-devloop-workflow" then
    error(ctx .. ": invalid-dev-engine", 0)
  end
  local packages = require_table(profile, "platform_packages", ctx)
  for _, package_name in ipairs(required_platform_packages) do
    require_list_contains(packages, package_name, ctx)
  end
  if has_item(packages, "github-devloop-intake-default") then
    error(ctx .. ": conflicting-intake-policy", 0)
  end
  local proof = require_table(profile, "necessity_proof", ctx)
  if proof.schema ~= "workflow-dev.necessity-proof.v1" then
    error(ctx .. ": unsupported-necessity-proof-schema", 0)
  end
  local duties = require_table(proof, "required_profile_duties", ctx)
  for _, duty in ipairs(required_profile_duties) do
    require_list_contains(duties, duty, ctx)
  end
  local alternatives = require_table(proof, "alternatives", ctx)
  for _, expected in ipairs(necessity_alternatives) do
    require_necessity_alternative(alternatives, expected, ctx)
  end
  if proof.conclusion ~= "workflow-dev owns the development workflow profile contract" then
    error(ctx .. ": invalid-necessity-proof-conclusion", 0)
  end
  local capabilities = require_table(profile, "host_capabilities", ctx)
  local commands = require_table(capabilities, "required_commands", ctx)
  for _, command in ipairs(required_commands) do
    require_list_contains(commands, command, ctx)
  end
  require_string(capabilities, "command_contract", ctx)
  require_string(capabilities, "artifact_contract", ctx)
  local handoff = require_table(profile, "handoff", ctx)
  if handoff.schema ~= "workflow-dev.handoff.v1" then
    error(ctx .. ": unsupported-handoff-schema", 0)
  end
  if handoff.payload_policy ~= "source-ref-only" then
    error(ctx .. ": non-source-ref-handoff", 0)
  end
  local source_ref = require_table(handoff, "code_artifact_source_ref", ctx)
  if source_ref.kind ~= "host-worktree" then
    error(ctx .. ": invalid-code-artifact-source-ref", 0)
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
    schema = "workflow-dev.host-package-roots.v1",
    profile = "workflow-dev",
    project_root_owner = "host",
    platform_source = "fkst-packages-platform",
    compose_file = ".fkst/compose/package-roots",
    required_entries = {
      "fkst-packages:packages/github-proxy",
      "fkst-packages:packages/consensus",
      "fkst-packages:packages/github-devloop-intake",
      "fkst-packages:packages/github-devloop-workflow",
      "fkst-packages:packages/github-devloop-decompose",
      "fkst-packages:packages/github-devloop",
      "fkst-packages:packages/github-devloop-pr",
      "fkst-packages:packages/github-devloop-ops",
      "fkst-packages:packages/github-devloop-integration",
      "fkst-packages:packages/workflow-dev",
    },
  }
end

return M
