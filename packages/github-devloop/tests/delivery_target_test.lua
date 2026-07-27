local delivery_target = require("devloop.delivery_target")
local forge_git = require("forge.git")
local builders = require("devloop.markers.builders")
local facts = require("devloop.markers.facts")
local devloop_base = require("devloop.base")
local entity = require("devloop.entity")
local replay_required_facts = require("devloop.replay_required_facts")
local t = fkst.test

local function grant_json(extra)
  return '[{' .. table.concat({
    '"lifecycle_repo":"owner/lifecycle"',
    '"lifecycle_issue":42',
    '"implementation_repo":"owner/implementation"',
    '"implementation_branch":"fkst-hosted"',
    '"implementation_root":"/runtime/platform"',
    extra,
  }, ","):gsub(",+$", "") .. '}]'
end

local function expect_error(fragment, fn)
  local ok, err = pcall(fn)
  assert(ok == false, "expected an error containing " .. fragment)
  assert(tostring(err):find(fragment, 1, true) ~= nil, tostring(err))
end

local function verified_exec(seen, overrides)
  return function(opts)
    table.insert(seen, opts.argv)
    local command = table.concat(opts.argv, " ")
    if command:find("rev-parse --show-toplevel", 1, true) ~= nil then
      return { stdout = (overrides and overrides.root or "/runtime/platform") .. "\n", stderr = "", exit_code = 0 }
    end
    if command:find("remote get-url origin", 1, true) ~= nil then
      return { stdout = (overrides and overrides.origin or "https://github.com/owner/implementation.git") .. "\n", stderr = "", exit_code = 0 }
    end
    if command:find("rev-parse --abbrev-ref HEAD", 1, true) ~= nil then
      return { stdout = (overrides and overrides.branch or "fkst-hosted") .. "\n", stderr = "", exit_code = 0 }
    end
    return { stdout = "", stderr = "", exit_code = 0 }
  end
end

return {
  test_exact_grant_resolves_and_verifies_scoped_checkout = function()
    local seen = {}
    local target = delivery_target.resolve("owner/lifecycle", 42, {
      raw = grant_json(),
      git_factory = function(root)
        return forge_git.scoped(verified_exec(seen), root)
      end,
    })
    assert(target.lifecycle_repo == "owner/lifecycle")
    assert(target.implementation_repo == "owner/implementation")
    assert(target.implementation_branch == "fkst-hosted")
    assert(target.implementation_root == "/runtime/platform")
    assert(target.cross_repo == true)
    assert(type(target.git) == "table")
    assert(#seen == 3)
    for _, argv in ipairs(seen) do
      assert(argv[1] == "git" and argv[2] == "-C" and argv[3] == "/runtime/platform")
    end
  end,

  test_scoped_git_prefixes_repository_level_argv = function()
    local seen
    local git = forge_git.scoped(function(opts)
      seen = opts.argv
      return { stdout = "", stderr = "", exit_code = 0 }
    end, "/runtime/platform")
    git.fetch_branch("origin", "fkst-hosted", 30)
    assert(table.concat(seen, "\0") == table.concat({
      "git", "-C", "/runtime/platform", "fetch", "origin", "fkst-hosted",
    }, "\0"))
  end,

  test_absent_grant_preserves_same_repository_behavior = function()
    local default_git = {}
    local target = delivery_target.resolve("owner/lifecycle", 42, {
      raw = "",
      verify = false,
      default_branch = "dev",
      default_git = default_git,
    })
    assert(target.cross_repo == false)
    assert(target.implementation_repo == "owner/lifecycle")
    assert(target.implementation_branch == "dev")
    assert(target.git == default_git)
  end,

  test_asserted_cross_repo_requires_exact_grant = function()
    expect_error("delivery-grant-missing", function()
      delivery_target.resolve("owner/lifecycle", 42, {
        raw = "",
        verify = false,
        implementation_repo = "owner/implementation",
      })
    end)
    expect_error("delivery-grant-mismatch", function()
      delivery_target.resolve("owner/lifecycle", 42, {
        raw = grant_json(),
        verify = false,
        implementation_repo = "owner/other",
      })
    end)
    expect_error("delivery-grant-mismatch", function()
      delivery_target.resolve("owner/lifecycle", 42, {
        raw = grant_json(),
        verify = false,
        implementation_repo = "owner/implementation",
        implementation_branch = "main",
      })
    end)
  end,

  test_grant_parser_rejects_malformed_ambiguous_and_unsafe_input = function()
    for _, raw in ipairs({
      "{}",
      "not-json",
      grant_json('"unknown":true'),
      grant_json():gsub('"lifecycle_issue":42', '"lifecycle_issue":0'),
      grant_json():gsub('"implementation_repo":"owner/implementation"', '"implementation_repo":"owner/lifecycle"'),
      grant_json():gsub('"implementation_branch":"fkst%-hosted"', '"implementation_branch":"bad branch"'),
      grant_json():gsub('"implementation_root":"/runtime/platform"', '"implementation_root":"/runtime/../secret"'),
    }) do
      expect_error("delivery-grant-invalid", function() delivery_target.parse(raw) end)
    end
    local duplicate = grant_json():sub(1, -2) .. "," .. grant_json():sub(2)
    expect_error("duplicate lifecycle identity", function() delivery_target.parse(duplicate) end)
  end,

  test_checkout_verification_fails_closed_without_disclosing_origin = function()
    expect_error("checkout origin differs from grant", function()
      delivery_target.resolve("owner/lifecycle", 42, {
        raw = grant_json(),
        git_factory = function(root)
          return forge_git.scoped(verified_exec({}, { origin = "https://credential@example.invalid/secret.git" }), root)
        end,
      })
    end)
    expect_error("checkout branch differs from grant", function()
      delivery_target.resolve("owner/lifecycle", 42, {
        raw = grant_json(),
        git_factory = function(root)
          return forge_git.scoped(verified_exec({}, { branch = "develop" }), root)
        end,
      })
    end)
  end,

  test_cross_repo_markers_round_trip_both_identities = function()
    local issue_proposal = "github-devloop/issue/owner/lifecycle/42"
    local pr_proposal = "github-devloop/pr/owner/implementation/7"
    local link = builders.pr_link_marker(issue_proposal, 7, "devloop/issue/42", "v1", "fkst-hosted", "owner/implementation")
    local delegation = builders.pr_delegation_marker(issue_proposal, pr_proposal, 7, "v1", "g1", "owner/implementation")
    local origin = builders.pr_origin_marker(issue_proposal, 42, "devloop/issue/42", "v1", "fkst-hosted", "owner/implementation")

    local link_fact = facts.pr_link_fact({ link }, issue_proposal)
    local delegation_fact = facts.pr_delegation_fact({ delegation }, issue_proposal, "v1")
    local origin_fact = facts.pr_origin_fact({ origin })
    for _, fact in ipairs({ link_fact, delegation_fact, origin_fact }) do
      assert(fact.lifecycle_repo == "owner/lifecycle")
      assert(fact.implementation_repo == "owner/implementation")
      assert(fact.cross_repo == true)
    end
  end,

  test_same_repo_marker_bytes_remain_unchanged = function()
    local proposal = "github-devloop/issue/owner/repo/42"
    assert(builders.pr_link_marker(proposal, 7, "feature", "v1", "dev")
      == '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. proposal .. '" pr="7" branch="feature" impl_version="v1" base_branch="dev" -->')
    assert(builders.pr_delegation_marker(proposal, "github-devloop/pr/owner/repo/7", 7, "v1", "g1")
      == '<!-- fkst:github-devloop:pr-delegation:v1 proposal="' .. proposal .. '" pr_proposal="github-devloop/pr/owner/repo/7" pr="7" version="v1" delegation="g1" -->')
    assert(builders.pr_origin_marker(proposal, 42, "feature", "v1", "dev")
      == '<!-- fkst:github-devloop:pr-origin:v1 proposal="' .. proposal .. '" issue="42" branch="feature" impl_version="v1" base_branch="dev" -->')
  end,

  test_restart_snapshot_selects_repository_qualified_pr_when_numbers_collide = function()
    local lifecycle_pr = { head_sha = "1111111111111111111111111111111111111111" }
    local implementation_pr = { head_sha = "2222222222222222222222222222222222222222" }
    local snapshot = {
      prs = {
        { repo = "owner/lifecycle", number = 7, current = lifecycle_pr },
        { repo = "owner/implementation", number = 7, current = implementation_pr },
      },
    }

    assert(replay_required_facts.find_linked_pr(snapshot, 7, "owner/implementation") == implementation_pr)
    assert(replay_required_facts.find_linked_pr(snapshot, 7, "owner/lifecycle") == lifecycle_pr)
    assert(replay_required_facts.find_linked_pr(snapshot, 7, "owner/missing") == nil)
  end,

  test_legacy_same_repo_link_is_not_reinterpreted_by_later_cross_repo_grant = function()
    local proposal = "github-devloop/issue/owner/lifecycle/42"
    local link = builders.pr_link_marker(proposal, 7, "feature", "v1", "dev")
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_DELIVERY_GRANTS"), {
      stdout = grant_json(),
      stderr = "",
      exit_code = 0,
    })
    local observed = {}
    expect_error("delivery-grant-mismatch", function()
      entity.linked_pr_surface_snapshot({
        _max_dedup_len = 512,
        git = {},
        gh_pr_view_observe = function(repo, pr_number)
          table.insert(observed, tostring(repo) .. "#" .. tostring(pr_number))
          return { stdout = "", stderr = "", exit_code = 1 }
        end,
      }, "owner/lifecycle", proposal, { link })
    end)
    assert(#observed == 0)
  end,
}
