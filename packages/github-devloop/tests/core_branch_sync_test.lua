local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

return {
  test_branch_sync_identity_helpers = function()
    local source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev")
    t.eq(source_ref.kind, "external")
    t.eq(source_ref.ref, "owner/repo#branch-sync/dev/integration/dev")

    t.eq(
      core.branch_sync_lock_key("owner/repo", "dev", "integration/dev"),
      "github-devloop/branch-sync/owner/repo/dev/integration/dev"
    )
    t.eq(
      core.repo_ref_store_lock_key("owner/repo"),
      "github-devloop/git/owner/repo/fetch"
    )
    t.eq(
      core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "abcdef1234"),
      "branch-sync/owner/repo/dev/integration/dev/abcdef1234"
    )
    t.eq(
      core.sync_commit_marker("owner/repo", "dev", "integration/dev", "abcdef1234", "fedcba4321", "clean"),
      '<!-- fkst:github-devloop:sync:v1 repo="owner/repo" upstream="dev" integration="integration/dev" upstream_sha="abcdef1234" integration_parent="fedcba4321" result="clean" -->'
    )
    local message = core.sync_commit_message("owner/repo", "dev", "integration/dev", "abcdef1234", "fedcba4321", "resolved")
    t.is_true(message:find("Sync dev into integration/dev", 1, true) == 1)
    t.is_true(message:find('result="resolved"', 1, true) ~= nil)

    t.eq(
      core.is_supported_sync_conflict({
        schema = "github-devloop.v1",
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        upstream_sha = "abcdef1234",
        integration_sha = "fedcba4321",
        dedup_key = "branch-sync/owner/repo/dev/integration/dev/abcdef1234",
        source_ref = source_ref,
      }),
      true
    )
  end,

  test_branch_sync_rejects_unsafe_shapes = function()
    t.raises(function()
      core.branch_sync_lock_key("../repo", "dev", "integration/dev")
    end)
    t.raises(function()
      core.repo_ref_store_lock_key("../repo")
    end)
    t.raises(function()
      core.branch_sync_source_ref("owner/repo", "../dev", "integration/dev")
    end)
    t.raises(function()
      core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "not-a-sha")
    end)
    t.raises(function()
      core.sync_commit_marker("owner/repo", "dev", "integration/dev", "abcdef", "fedcba", "manual")
    end)
  end,

  test_branch_scan_departments_serialize_fetches_on_repo_ref_store_lock = function()
    local expected = "core.with_repo_ref_store_lock(repo"
    for _, path in ipairs({
      "packages/github-devloop/departments/sync_scan/main.lua",
      "packages/github-devloop/departments/rollup_scan/main.lua",
      "packages/github-devloop/departments/pr_freshness_scan/main.lua",
      "packages/github-devloop/departments/sync_conflict/main.lua",
    }) do
      local text = file.read(path)
      t.is_true(text:find(expected, 1, true) ~= nil)
      t.is_true(text:find("git_fetch_branch_cmd", 1, true) ~= nil)
    end
  end,
}
