local h = require("tests.devloop_helpers")
local t = h.t
local github_risk = require("devloop.github_risk")

return {
  test_scheduler_and_workflow_surfaces_are_high_risk = function()
    t.eq(github_risk.github_high_risk_path("packages/github-devloop-pr/raisers/merge_scan.lua"), true)
    t.eq(github_risk.github_high_risk_path("packages/github-devloop/raisers/sync.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/workflow/saga.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/workflow/liveness/contract.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/devloop/claims.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/devloop/config.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/forge/github/exec.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/forge/git/exec.lua"), true)
    -- structural forge-authority coverage: top-level entrypoints + merge authority,
    -- not only enumerated subdirs (the prior denylist missed these).
    t.eq(github_risk.github_high_risk_path("libraries/forge/github.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/forge/git.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/forge/merge_commands.lua"), true)
    t.eq(github_risk.github_high_risk_path("libraries/forge/merge/verified_merge.lua"), true)
  end,
}
