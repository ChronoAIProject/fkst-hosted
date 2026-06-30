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
  test_default_profile_declares_existing_devloop_packages = function()
    local profile = core.default_profile()

    t.eq(profile.schema, "frontend-devloop.profile.v1")
    t.eq(profile.name, "frontend-devloop")
    t.eq(profile.owns, "host-ui-application-workflow-profile")
    t.eq(profile.browser_qa_owner, "browser-qa")
    t.eq(profile.issue_lifecycle_owner, "github-devloop")

    local packages = profile.platform_packages
    t.eq(packages[1], "github-proxy")
    t.eq(packages[2], "consensus")
    t.eq(packages[3], "github-devloop-intake")
    t.eq(packages[4], "github-devloop-intake-default")
    t.eq(packages[5], "github-devloop-decompose")
    t.eq(packages[6], "github-devloop")
    t.eq(packages[7], "github-devloop-pr")
    t.eq(packages[8], "github-devloop-ops")
    t.eq(packages[9], "github-devloop-integration")
    t.eq(packages[10], "frontend-devloop")

    t.is_true(list_contains(profile.host_capabilities.required_commands, "install"))
    t.is_true(list_contains(profile.host_capabilities.required_commands, "lint"))
    t.is_true(list_contains(profile.host_capabilities.required_commands, "test"))
    t.is_true(list_contains(profile.host_capabilities.required_commands, "build"))
  end,

  test_default_profile_proves_why_frontend_devloop_owns_the_profile = function()
    local proof = core.default_profile().necessity_proof

    t.eq(proof.schema, "frontend-devloop.necessity-proof.v1")
    t.eq(proof.conclusion, "frontend-devloop owns the UI workflow profile contract")

    local scripts = find_by_surface(proof.alternatives, "project-local scripts")
    t.eq(scripts.owner, "host")
    t.eq(scripts.insufficiency, "commands do not declare fkst package roots or trust boundaries")

    local browser_qa = find_by_surface(proof.alternatives, "browser-qa")
    t.eq(browser_qa.owner, "browser-qa")
    t.eq(browser_qa.insufficiency, "browser execution does not own devloop package composition")

    local global_host = find_by_surface(proof.alternatives, "global-host profiles")
    t.eq(global_host.owner, "host profile layer")
    t.eq(global_host.insufficiency, "generic host hydration does not own UI workflow artifact handoff")
  end,

  test_necessity_proof_compares_existing_surfaces_to_profile_duties = function()
    local proof = core.default_profile().necessity_proof

    assert_contains_all(proof.required_profile_duties, {
      "reusable platform package composition",
      "UI workflow trust-boundary declaration",
      "source-ref-only UI artifact handoff",
      "package-local conformance for the UI profile contract",
    })

    local scripts = find_by_surface(proof.alternatives, "project-local scripts")
    assert_contains_all(scripts.existing_surfaces, {
      "package-manager scripts",
      ".fkst/compose/package-roots",
    })
    assert_contains_all(scripts.can_express, {
      "host-owned command execution",
      "host-local package root selection",
    })
    assert_contains_all(scripts.missing_profile_duties, {
      "reusable platform package composition",
      "UI workflow trust-boundary declaration",
      "package-local conformance for the UI profile contract",
    })
    t.eq(scripts.boundary_violation, "Host-local files would make each frontend host duplicate platform semantics that fkst-packages should validate once.")

    local browser_qa = find_by_surface(proof.alternatives, "browser-qa")
    assert_contains_all(browser_qa.can_express, {
      "browser execution",
      "visual validation",
    })
    assert_contains_all(browser_qa.missing_profile_duties, {
      "reusable platform package composition",
      "GitHub devloop lifecycle ownership",
    })
    t.eq(browser_qa.boundary_violation, "Putting package composition in browser-qa would couple browser execution to GitHub issue-to-PR lifecycle orchestration.")

    local global_host = find_by_surface(proof.alternatives, "global-host profiles")
    assert_contains_all(global_host.can_express, {
      "generic host hydration",
      "workspace-root wiring",
    })
    assert_contains_all(global_host.missing_profile_duties, {
      "UI workflow trust-boundary declaration",
      "source-ref-only UI artifact handoff",
    })
    t.eq(global_host.boundary_violation, "Putting UI artifact trust policy in the global host layer would couple generic host hydration to frontend workflow semantics.")
  end,

  test_necessity_proof_rules_out_existing_surface_ownership = function()
    local proof = core.default_profile().necessity_proof

    local scripts = find_by_surface(proof.alternatives, "project-local scripts")
    t.eq(scripts.ownership_conflict.actual_surface, ".fkst/compose/package-roots")
    t.eq(scripts.ownership_conflict.current_authority, "host-local package root selection")
    assert_contains_all(scripts.ownership_conflict.must_not_own, {
      "UI workflow trust-boundary declaration",
      "source-ref-only UI artifact handoff",
    })
    t.eq(scripts.ownership_conflict.reason, ".fkst/compose/package-roots is host input for selected roots, not the package-owned UI workflow profile authority.")

    local browser_qa = find_by_surface(proof.alternatives, "browser-qa")
    t.eq(browser_qa.ownership_conflict.actual_surface, "browser-qa")
    t.eq(browser_qa.ownership_conflict.current_authority, "browser execution and visual validation")
    assert_contains_all(browser_qa.ownership_conflict.must_not_own, {
      "reusable platform package composition",
      "GitHub devloop lifecycle ownership",
    })
    t.eq(browser_qa.ownership_conflict.reason, "browser-qa validates UI runtime behavior, not the package graph or issue-to-PR lifecycle.")

    local global_host = find_by_surface(proof.alternatives, "global-host profiles")
    t.eq(global_host.ownership_conflict.actual_surface, "global host profile environment")
    t.eq(global_host.ownership_conflict.current_authority, "machine-local environment and workspace wiring")
    assert_contains_all(global_host.ownership_conflict.must_not_own, {
      "UI workflow trust-boundary declaration",
      "source-ref-only UI artifact handoff",
    })
    t.eq(global_host.ownership_conflict.reason, "global host profiles intentionally exclude package roots and frontend workflow semantics.")
  end,

  test_default_profile_uses_source_refs_for_ui_artifacts = function()
    local profile = core.default_profile()
    local handoff = profile.handoff

    t.eq(handoff.schema, "frontend-devloop.handoff.v1")
    t.eq(handoff.payload_policy, "source-ref-only")
    t.eq(handoff.ui_artifact_source_ref.kind, "host-worktree")
    t.eq(handoff.ui_artifact_source_ref.ref, "host://ui-artifacts")
    t.is_true(list_contains(handoff.trust_boundaries, "issue-content-untrusted"))
    t.is_true(list_contains(handoff.trust_boundaries, "browser-results-untrusted"))
    t.is_true(list_contains(handoff.trust_boundaries, "host-scripts-owned-by-host"))
  end,

  test_validate_profile_rejects_missing_packages_or_payload_snapshot_policy = function()
    local missing_package = core.default_profile()
    table.remove(missing_package.platform_packages, 2)
    t.raises(function()
      core.validate_profile(missing_package)
    end)

    local snapshot_payload = core.default_profile()
    snapshot_payload.handoff.payload_policy = "embed-ui-artifacts"
    t.raises(function()
      core.validate_profile(snapshot_payload)
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
    t.eq(roots.profile, "frontend-devloop")
    t.eq(roots.project_root_owner, "host")
    t.eq(roots.platform_source, "fkst-packages-platform")
    t.eq(roots.compose_file, ".fkst/compose/package-roots")
    t.is_true(list_contains(roots.required_entries, "fkst-packages:packages/frontend-devloop"))
  end,
}
