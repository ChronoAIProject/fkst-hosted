local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
local devloop_base = require("devloop.base")

local repo = "owner/repo"
local dependent_number = 42
local blocker_number = 61
local child_pr_number = 62
local dependent_proposal = "github-devloop/issue/owner/repo/42"
local blocker_proposal = "github-devloop/issue/owner/repo/61"
local child_pr_proposal = "github-devloop/pr/owner/repo/62"
local dependent_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local blocker_version = "ready/consensus-github-devloop/issue/owner/repo/61/2026-06-03T01-02-03Z"
local child_head_sha = "0123456789abcdef0123456789abcdef01234567"

local function blocked_by_json(nodes)
  local rendered = {}
  for _, node in ipairs(nodes or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"state":"%s","stateReason":"%s","repository":{"nameWithOwner":"%s"}}',
      tonumber(node.number),
      tostring(node.state or "OPEN"),
      tostring(node.state_reason or node.stateReason or ""),
      tostring(node.repo or repo)
    ))
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":'
    .. tostring(#rendered)
    .. ',"pageInfo":{"hasNextPage":false},"nodes":['
    .. table.concat(rendered, ",")
    .. "]}}}}}\n"
end

local function mock_blocked_by(issue_number, nodes)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = blocked_by_json(nodes),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_dependent_issue()
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = dependent_number,
    labels = { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
    comments = {
      core.state_marker(dependent_proposal, "dependency_wait", dependent_version),
      core.dependency_wait_marker(dependent_proposal, dependent_version, { blocker_number }),
    },
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  })
end

local function mock_delegated_blocker_issue(options)
  local opts = options or {}
  local pr_repo = opts.implementation_repo or repo
  local pr_proposal = "github-devloop/pr/" .. pr_repo .. "/" .. tostring(child_pr_number)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = blocker_number,
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = {
      core.state_marker(blocker_proposal, "awaiting-pr", blocker_version),
      m_builders.pr_delegation_marker(
        blocker_proposal,
        pr_proposal,
        child_pr_number,
        blocker_version,
        "g1",
        opts.implementation_repo
      ),
    },
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,comments,state,stateReason,assignees,author")
end

local function mock_merged_child_pr(options)
  local opts = options or {}
  local pr_repo = opts.implementation_repo or repo
  local base_branch = opts.base_branch or "dev"
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = pr_repo,
    number = child_pr_number,
    state = "MERGED",
    head = "devloop-owner-repo-61-01HY",
    head_sha = child_head_sha,
    base_branch = base_branch,
    comments = {
      m_builders.pr_origin_marker(
        blocker_proposal,
        blocker_number,
        "devloop-owner-repo-61-01HY",
        blocker_version,
        base_branch,
        opts.implementation_repo
      ),
      core.state_marker(blocker_proposal, "merged", blocker_version),
      m_builders.merged_marker(core, blocker_proposal, child_pr_number, blocker_version, child_head_sha),
    },
  }, entity_read_mocks.pr_origin_selector)
end

local function find_raise(raises, queue, predicate)
  for _, item in ipairs(raises or {}) do
    if item.queue == queue and (predicate == nil or predicate(item.payload or {})) then
      return item
    end
  end
  return nil
end

local function ready_handoff_raise(raises)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return type(payload.handoff) == "table"
      and payload.handoff.kind == "github-devloop.ready"
  end)
end

local function has_marker(raises, marker_text)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(marker_text, 1, true) ~= nil
  end) ~= nil
end

return {
  test_dependency_wait_releases_when_blocker_delegated_pr_is_merged = function()
    mock_dependent_issue()
    mock_blocked_by(dependent_number, {
      { number = blocker_number, state = "CLOSED", state_reason = "COMPLETED" },
    })
    mock_delegated_blocker_issue()
    mock_merged_child_pr()

    local result = h.run_department("departments/observe_issue/main.lua", {
      queue = "github-proxy.github_issue_changed",
      payload = h.issue({
        number = dependent_number,
        labels = { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      }),
    }, h.opts("dependency-pr-delegation-cascade"))

    t.eq(result.exit_code, 0)
    t.is_true(ready_handoff_raise(result.raises) ~= nil)
    t.is_true(has_marker(result.raises, "fkst:github-devloop:dependency-release:v1"))
  end,

  test_legacy_cross_repo_delegation_without_explicit_delivery_identity_is_rejected = function()
    local legacy_pr_proposal = "github-devloop/pr/owner/implementation/" .. tostring(child_pr_number)
    local comments = {
      core.state_marker(blocker_proposal, "awaiting-pr", blocker_version),
      m_builders.pr_delegation_marker(
        blocker_proposal,
        legacy_pr_proposal,
        child_pr_number,
        blocker_version,
        "g1"
      ),
    }

    local merged, reason = core.delegated_blocker_merged(repo, blocker_number, blocker_proposal, {
      comments = comments,
    }, {
      state = "awaiting-pr",
      version = blocker_version,
    })

    t.eq(merged, nil)
    t.eq(reason, "pr-delegation-mismatch")
  end,

  test_dependency_recovery_reads_explicit_cross_repo_delegated_pr_after_restart = function()
    local implementation_repo = "owner/implementation"
    local grant = '[{"lifecycle_repo":"owner/repo","lifecycle_issue":61,'
      .. '"implementation_repo":"' .. implementation_repo .. '",'
      .. '"implementation_branch":"fkst-hosted","implementation_root":"/runtime/implementation"}]'
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_DELIVERY_GRANTS"), {
      stdout = grant,
      stderr = "",
      exit_code = 0,
    })
    mock_dependent_issue()
    mock_blocked_by(dependent_number, {
      { number = blocker_number, state = "CLOSED", state_reason = "COMPLETED" },
    })
    mock_delegated_blocker_issue({ implementation_repo = implementation_repo })
    mock_merged_child_pr({
      implementation_repo = implementation_repo,
      base_branch = "fkst-hosted",
    })

    local result = h.run_department("departments/observe_issue/main.lua", {
      queue = "github-proxy.github_issue_changed",
      payload = h.issue({
        number = dependent_number,
        labels = { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      }),
    }, h.opts("dependency-cross-repo-delegation-restart", {
      FKST_DEVLOOP_DELIVERY_GRANTS = grant,
    }))

    t.eq(result.exit_code, 0)
    t.is_true(ready_handoff_raise(result.raises) ~= nil)
    t.is_true(has_marker(result.raises, "fkst:github-devloop:dependency-release:v1"))
  end,
}
