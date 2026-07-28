local base_ids = require("devloop.base_ids")
local h = require("tests.devloop_helpers")
local core = h.core
local t = h.t

local repo = "owner/repo"

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function comment_json(body, author)
  return '{"body":"' .. json_string(body)
    .. '","author":{"login":"' .. json_string(author or "fkst-test-bot")
    .. '"},"createdAt":"2026-07-28T00:00:00Z"}'
end

local function blocked_by_json(nodes)
  local rendered = {}
  for _, node in ipairs(nodes or {}) do
    rendered[#rendered + 1] = '{"number":' .. tostring(node.number)
      .. ',"state":"' .. tostring(node.state or "OPEN")
      .. '","stateReason":"' .. tostring(node.state_reason or "")
      .. '","repository":{"nameWithOwner":"' .. tostring(node.repo or repo) .. '"}}'
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":'
    .. tostring(#rendered)
    .. ',"pageInfo":{"hasNextPage":false},"nodes":['
    .. table.concat(rendered, ",")
    .. ']}}}}}'
end

local function mock_blocked_by(issue_number, nodes)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = blocked_by_json(nodes),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue(issue_number, comments, state)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    rendered[#rendered + 1] = comment_json(comment.body or comment, comment.author)
  end
  t.mock_command(core.gh_issue_view_observe_cmd(repo, issue_number), {
    stdout = '{"state":"' .. tostring(state or "OPEN")
      .. '","comments":[' .. table.concat(rendered, ",")
      .. '],"author":{"login":"fkst-test-bot"}}',
    stderr = "",
    exit_code = 0,
  })
end

local function origin_context(child_number, origin_number, version, author)
  local child = base_ids.proposal_id(repo, child_number)
  local origin = base_ids.proposal_id(repo, origin_number)
  return {
    proposal_id = child,
    version = version,
    comments = {
      {
        body = core.state_marker(child, "thinking", version)
          .. "\n" .. core.dependency_origin_marker(child, version, origin),
        author_login = author or "fkst-test-bot",
      },
    },
  }
end

local function workflow_terminal(origin_number, state)
  return '<!-- fkst:github-devloop-workflow:terminal:v1 origin="'
    .. base_ids.proposal_id(repo, origin_number)
    .. '" state="' .. state
    .. '" reason_code="all-slots-result-ready" -->'
end

return {
  test_workflow_child_waits_on_its_origins_native_dependencies = function()
    local child, origin, blocker = 142, 140, 139
    mock_blocked_by(child, {})
    mock_blocked_by(origin, { { number = blocker } })
    mock_blocked_by(blocker, {})
    mock_issue(blocker, {
      core.state_marker(base_ids.proposal_id(repo, blocker), "ready", "v-139"),
    })

    local gate = core.dependency_gate(repo, child, origin_context(child, origin, "v-child"))

    t.eq(gate.ok, false)
    t.eq(gate.kind, "waiting")
    t.eq(gate.unmet[1], blocker)
  end,

  test_workflow_child_releases_when_its_origins_dependencies_merge = function()
    local child, origin, blocker = 152, 150, 149
    mock_blocked_by(child, {})
    mock_blocked_by(origin, { { number = blocker } })
    mock_blocked_by(blocker, {})
    mock_issue(blocker, {
      core.state_marker(base_ids.proposal_id(repo, blocker), "merged", "v-149"),
    })

    local gate = core.dependency_gate(repo, child, origin_context(child, origin, "v-child"))

    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_conflicting_or_cross_repo_dependency_origins_fail_closed = function()
    local child = 162
    local proposal = base_ids.proposal_id(repo, child)
    local version = "v-child"
    local context = origin_context(child, 160, version)
    context.comments[#context.comments + 1] = {
      body = core.state_marker(proposal, "thinking", version)
        .. "\n" .. core.dependency_origin_marker(
          proposal,
          version,
          base_ids.proposal_id(repo, 161)
        ),
      author_login = "fkst-test-bot",
    }

    local conflict = core.dependency_gate(repo, child, context)
    t.eq(conflict.ok, false)
    t.eq(conflict.kind, "unresolvable")
    t.eq(conflict.reason, "dependency-origin-conflict")

    local cross_repo = origin_context(child, 160, version)
    cross_repo.comments[1].body = core.state_marker(proposal, "thinking", version)
      .. "\n" .. core.dependency_origin_marker(
        proposal,
        version,
        base_ids.proposal_id("other/repo", 160)
      )
    local cross = core.dependency_gate(repo, child, cross_repo)
    t.eq(cross.ok, false)
    t.eq(cross.reason, "cross-repo-dependency-origin")
  end,

  test_untrusted_dependency_origin_marker_cannot_change_an_ordinary_gate = function()
    local child = 172
    mock_blocked_by(child, {})

    local gate = core.dependency_gate(repo, child, origin_context(child, 170, "v-child", "human"))

    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_ordinary_consensus_version_without_origin_marker_is_unchanged = function()
    local child = 175
    mock_blocked_by(child, {})

    local gate = core.dependency_gate(repo, child, {
      proposal_id = base_ids.proposal_id(repo, child),
      version = "consensus:github-devloop/issue/owner/repo/175/intake/v1",
      comments = {
        {
          body = core.state_marker(
            base_ids.proposal_id(repo, child),
            "thinking",
            "consensus:github-devloop/issue/owner/repo/175/intake/v1"
          ),
          author_login = "fkst-test-bot",
        },
      },
    })

    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_dependency_origin_survives_a_ready_split_version = function()
    local child, origin, blocker = 176, 174, 173
    local context = origin_context(child, origin, "v-child")
    context.version = core.ready_split_version(context.version)
    mock_blocked_by(child, {})
    mock_blocked_by(origin, { { number = blocker } })
    mock_blocked_by(blocker, {})
    mock_issue(blocker, {
      core.state_marker(base_ids.proposal_id(repo, blocker), "ready", "v-173"),
    })

    local gate = core.dependency_gate(repo, child, context)

    t.eq(gate.ok, false)
    t.eq(gate.kind, "waiting")
    t.eq(gate.unmet[1], blocker)
  end,

  test_dependency_origin_version_mismatch_fails_closed = function()
    local child, origin = 178, 177
    local context = origin_context(child, origin, "old-intake")
    context.version = "new-intake"

    local gate = core.dependency_gate(repo, child, context)

    t.eq(gate.ok, false)
    t.eq(gate.kind, "unresolvable")
    t.eq(gate.reason, "dependency-origin-version-mismatch")
  end,

  test_dependency_origin_without_its_thinking_transition_fails_closed = function()
    local child, origin = 188, 187
    local context = origin_context(child, origin, "v-child")
    context.comments[1].body = core.dependency_origin_marker(
      context.proposal_id,
      context.version,
      base_ids.proposal_id(repo, origin)
    )

    local gate = core.dependency_gate(repo, child, context)

    t.eq(gate.ok, false)
    t.eq(gate.kind, "unresolvable")
    t.eq(gate.reason, "dependency-origin-invalid")
  end,

  test_trusted_workflow_done_fact_satisfies_a_native_blocker = function()
    local dependent, workflow_origin = 182, 180
    mock_blocked_by(dependent, {
      { number = workflow_origin, state = "CLOSED", state_reason = "COMPLETED" },
    })
    mock_issue(workflow_origin, { workflow_terminal(workflow_origin, "done") }, "CLOSED")

    local gate = core.dependency_gate(repo, dependent)

    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_non_done_or_untrusted_workflow_terminal_cannot_satisfy_a_blocker = function()
    local blocked_dependent, blocked_origin = 192, 190
    mock_blocked_by(blocked_dependent, {
      { number = blocked_origin, state = "CLOSED", state_reason = "COMPLETED" },
    })
    mock_issue(blocked_origin, { workflow_terminal(blocked_origin, "blocked") }, "CLOSED")

    local blocked = core.dependency_gate(repo, blocked_dependent)
    t.eq(blocked.ok, false)
    t.eq(blocked.reason, "dependency-waiver-required")

    local forged_dependent, forged_origin = 202, 200
    mock_blocked_by(forged_dependent, {
      { number = forged_origin, state = "CLOSED", state_reason = "COMPLETED" },
    })
    mock_issue(forged_origin, {
      { body = workflow_terminal(forged_origin, "done"), author = "human" },
    }, "CLOSED")

    local forged = core.dependency_gate(repo, forged_dependent)
    t.eq(forged.ok, false)
    t.eq(forged.reason, "dependency-waiver-required")
  end,
}
