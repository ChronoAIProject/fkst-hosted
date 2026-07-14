local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")

local core = h.core
local t = h.t

local repo = "owner/repo"
local issue_number = 2275
local root_version = "ready/consensus-github-devloop/issue/owner/repo/2275/2026-07-14T01-02-03Z"
local replacement_version = root_version .. "/reimplement/1"

return {
  test_first_replacement_version_is_preserved_for_attempt_and_branch_selection = function()
    t.eq(core.implementation_attempt_version(replacement_version, nil), replacement_version)
    t.eq(core.implementation_attempt_version(replacement_version, 1), replacement_version)
    t.eq(core.implementation_branch_version(replacement_version, nil), replacement_version)
    t.eq(core.implementation_branch_version(replacement_version, 1), replacement_version)
  end,

  test_first_replacement_branch_differs_from_abandoned_original_branch = function()
    local original = devloop_base.implement_branch(repo, issue_number, root_version)
    local replacement = devloop_base.implement_branch(
      repo,
      issue_number,
      core.implementation_branch_version(replacement_version, 1)
    )

    t.is_true(replacement ~= original)
    t.is_true(replacement:find("reimplement-1", 1, true) ~= nil)
  end,
}
