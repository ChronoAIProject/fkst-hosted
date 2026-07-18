local core = require("core")
local t = fkst.test

local function list_contains(list, expected)
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

local function find_by_surface(rows, expected)
  for _, row in ipairs(rows) do
    if row.surface == expected then
      return row
    end
  end
  return nil
end

local function assert_contains_all(list, expected)
  for _, value in ipairs(expected) do
    t.is_true(list_contains(list, value))
  end
end

return {
  test_default_profile_selects_the_workflow_intake_policy = function()
    local profile = core.default_profile()

    t.eq(profile.schema, "workflow-dev.profile.v1")
    t.eq(profile.name, "workflow-dev")
    t.eq(profile.owns, "host-development-workflow-profile")
    t.eq(profile.issue_lifecycle_owner, "github-devloop")
    t.eq(profile.intake_policy_owner, "github-devloop-workflow")
    t.eq(profile.intake_policy_topology, "workflow")
    t.eq(profile.dev_engine, "github-devloop-workflow")
    t.eq(profile.shared_kernel, "workflow.engine")
  end,

  test_default_profile_composes_the_workflow_topology_devloop_family = function()
    local packages = core.default_profile().platform_packages

    t.eq(packages[1], "github-proxy")
    t.eq(packages[2], "consensus")
    t.eq(packages[3], "github-devloop-intake")
    t.eq(packages[4], "github-devloop-workflow")
    t.eq(packages[5], "github-devloop-decompose")
    t.eq(packages[6], "github-devloop")
    t.eq(packages[7], "github-devloop-pr")
    t.eq(packages[8], "github-devloop-ops")
    t.eq(packages[9], "github-devloop-integration")
    t.eq(packages[10], "workflow-dev")

    -- single intake-policy consumer: the default topology package is absent.
    t.is_true(not list_contains(packages, "github-devloop-intake-default"))

    local commands = core.default_profile().host_capabilities.required_commands
    assert_contains_all(commands, { "install", "lint", "test", "build" })
  end,

  test_default_profile_proves_why_workflow_dev_owns_the_profile = function()
    local proof = core.default_profile().necessity_proof

    t.eq(proof.schema, "workflow-dev.necessity-proof.v1")
    t.eq(proof.conclusion, "workflow-dev owns the development workflow profile contract")

    assert_contains_all(proof.required_profile_duties, {
      "reusable devloop platform composition",
      "workflow-engine intake policy selection",
      "development workflow trust-boundary declaration",
      "package-local conformance for the development profile contract",
    })

    local default_topology = find_by_surface(proof.alternatives, "default-topology profile")
    t.eq(default_topology.owner, "frontend-devloop")
    t.eq(default_topology.insufficiency, "the default topology selects github-devloop-intake-default, not the workflow engine intake policy")
    assert_contains_all(default_topology.missing_profile_duties, {
      "workflow-engine intake policy selection",
      "development workflow trust-boundary declaration",
    })

    local engine_pkg = find_by_surface(proof.alternatives, "github-devloop-workflow package")
    t.eq(engine_pkg.owner, "github-devloop-workflow")
    t.eq(engine_pkg.insufficiency, "the workflow engine package owns the intake seat and materialization, not the host development profile composition")
    assert_contains_all(engine_pkg.missing_profile_duties, {
      "reusable devloop platform composition",
      "development workflow trust-boundary declaration",
      "package-local conformance for the development profile contract",
    })
  end,

  test_default_profile_uses_source_refs_for_code_artifacts = function()
    local handoff = core.default_profile().handoff

    t.eq(handoff.schema, "workflow-dev.handoff.v1")
    t.eq(handoff.payload_policy, "source-ref-only")
    t.eq(handoff.code_artifact_source_ref.kind, "host-worktree")
    t.eq(handoff.code_artifact_source_ref.ref, "host://code-artifacts")
    t.is_true(list_contains(handoff.trust_boundaries, "issue-content-untrusted"))
    t.is_true(list_contains(handoff.trust_boundaries, "codex-results-untrusted"))
    t.is_true(list_contains(handoff.trust_boundaries, "host-scripts-owned-by-host"))
  end,

  test_validate_profile_accepts_the_default_profile = function()
    local profile = core.default_profile()
    t.eq(core.validate_profile(profile), profile)
  end,

  test_validate_profile_rejects_a_missing_platform_package = function()
    local missing_package = core.default_profile()
    table.remove(missing_package.platform_packages, 4)
    t.raises(function()
      core.validate_profile(missing_package)
    end)
  end,

  test_validate_profile_rejects_the_default_topology_intake_policy = function()
    local conflicting = core.default_profile()
    table.insert(conflicting.platform_packages, "github-devloop-intake-default")
    t.raises(function()
      core.validate_profile(conflicting)
    end)
  end,

  test_validate_profile_rejects_a_non_workflow_intake_policy = function()
    local wrong_policy = core.default_profile()
    wrong_policy.intake_policy_owner = "github-devloop-intake-default"
    t.raises(function()
      core.validate_profile(wrong_policy)
    end)
  end,

  test_validate_profile_rejects_an_embedded_artifact_payload = function()
    local embedded = core.default_profile()
    embedded.handoff.payload_policy = "embed-code-artifacts"
    t.raises(function()
      core.validate_profile(embedded)
    end)
  end,

  test_validate_profile_rejects_missing_necessity_proof = function()
    local missing_proof = core.default_profile()
    missing_proof.necessity_proof = nil
    t.raises(function()
      core.validate_profile(missing_proof)
    end)
  end,

  test_host_package_roots_contract_is_explicit = function()
    local roots = core.host_package_roots_contract()
    t.eq(roots.profile, "workflow-dev")
    t.eq(roots.project_root_owner, "host")
    t.eq(roots.platform_source, "fkst-packages-platform")
    t.eq(roots.compose_file, ".fkst/compose/package-roots")
    t.is_true(list_contains(roots.required_entries, "fkst-packages:packages/github-devloop-workflow"))
    t.is_true(list_contains(roots.required_entries, "fkst-packages:packages/workflow-dev"))
  end,
}
