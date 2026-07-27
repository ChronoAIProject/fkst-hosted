local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core

local lifecycle_repo = "owner/lifecycle"
local implementation_repo = "owner/implementation"
local proposal_id = "github-devloop/issue/owner/lifecycle/42"
local version = "ready/consensus-github-devloop/issue/owner/lifecycle/42/2026-06-03T01-02-03Z"

local function delivery_grant_json()
  return '[{"lifecycle_repo":"' .. lifecycle_repo
    .. '","lifecycle_issue":42,"implementation_repo":"' .. implementation_repo
    .. '","implementation_branch":"fkst-hosted","implementation_root":"/runtime/implementation"}]'
end

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
end

return {
  test_observe_pr_routes_cross_repo_origin_through_lifecycle_issue_and_implementation_pr = function()
    local grant = delivery_grant_json()
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_DELIVERY_GRANTS"), {
      stdout = grant,
      stderr = "",
      exit_code = 0,
    })
    h.mock_pr_origin_for({
      repo = implementation_repo,
      number = 7,
      head = "devloop-owner-lifecycle-42-01HY",
      head_sha = "def456",
      base_branch = "fkst-hosted",
      comments = {
        m_builders.pr_origin_marker(
          proposal_id,
          42,
          "devloop-owner-lifecycle-42-01HY",
          version,
          "fkst-hosted",
          implementation_repo
        ),
      },
      labels = { "fkst-dev:pr-open" },
    })
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = lifecycle_repo,
      number = 42,
      title = "Deliver to another repository",
      labels = { "fkst-dev:enabled", "fkst-dev:pr-open" },
      comments = {
        core.state_marker(proposal_id, "pr-open", version),
      },
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
      register_all_views = true,
    })

    local result = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = implementation_repo,
      number = 7,
      dedup_key = implementation_repo .. "#pr#7@2026-06-04T01:02:03Z",
      source_ref = entity_lib.pr_source_ref(implementation_repo, 7),
    }, h.opts("observe-pr-cross-repo", {
      FKST_DEVLOOP_DELIVERY_GRANTS = grant,
    }))

    t.eq(result.exit_code, 0, tostring(result.error or result.stderr or "cross-repository observe failed"))
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.eq(comment.payload.repo, implementation_repo)
    t.eq(comment.payload.pr_number, 7)
    t.eq(comment.payload.handoff.proposal_id, proposal_id)
    t.eq(comment.payload.handoff.source_ref.ref, implementation_repo .. "#pr/7")
  end,
}
