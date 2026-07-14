local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local entity_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
local payloads_builders = require("devloop.payloads.builders")

local t = h.t
local core = h.core

local REPO = "owner/repo"
local ISSUE_NUMBER = 2275
local PR_NUMBER = 2281
local BOT = "fkst-test-bot"
local INTEGRATION_BRANCH = "integration/dev"
local INTEGRATION_SHA = "1111111111111111111111111111111111111111"
local PR_HEAD_SHA = "2222222222222222222222222222222222222222"
local TERMINAL_COMMENT_ID = "2275001"
local READY_COMMENT_ID = "2275002"
local PROPOSAL_ID = "github-devloop/issue/owner/repo/2275"
local ROOT_VERSION = "ready/consensus-github-devloop/issue/owner/repo/2275/2026-07-14T01-02-03Z"
local REPLACEMENT_VERSION = ROOT_VERSION .. "/reimplement/1"
local PR_PROPOSAL_ID = entity_lib.pr_proposal_id(REPO, PR_NUMBER)
local ORIGINAL_BRANCH = devloop_base.implement_branch(REPO, ISSUE_NUMBER, ROOT_VERSION)
local REPLACEMENT_BRANCH = devloop_base.implement_branch(REPO, ISSUE_NUMBER, REPLACEMENT_VERSION)
local UNMERGED = "100644 abcdef 1\tpackages/github-devloop/core.lua\n"

local function comment(body, created_at, id)
  return {
    id = id or tostring(created_at or body):gsub("[^%w_%-]", "_"):sub(1, 60),
    body = body,
    author_login = BOT,
    created_at = created_at or "2026-07-14T01:03:00Z",
  }
end

local function delivery_source_ref(source_ref)
  return {
    kind = source_ref.kind,
    reference = source_ref.ref,
  }
end

local function parent_comments(version)
  return {
    comment(core.state_marker(PROPOSAL_ID, "awaiting-pr", version), "2026-07-14T01:03:00Z"),
    comment(m_builders.pr_delegation_marker(
      PROPOSAL_ID,
      PR_PROPOSAL_ID,
      PR_NUMBER,
      version,
      "g1"
    ), "2026-07-14T01:03:01Z"),
  }
end

local function pr_comments(state, version, branch)
  local impl_version = version or ROOT_VERSION
  return {
    comment(m_builders.pr_origin_marker(
      PROPOSAL_ID,
      ISSUE_NUMBER,
      branch or ORIGINAL_BRANCH,
      impl_version,
      INTEGRATION_BRANCH
    ), "2026-07-14T01:02:00Z"),
    comment(core.state_marker(PROPOSAL_ID, state, impl_version), "2026-07-14T01:04:00Z"),
  }
end

local function conflict_event(branch, version)
  local managed_branch = branch or ORIGINAL_BRANCH
  return {
    schema = "github-devloop.v1",
    repo = REPO,
    upstream_branch = INTEGRATION_BRANCH,
    integration_branch = managed_branch,
    upstream_sha = INTEGRATION_SHA,
    integration_sha = PR_HEAD_SHA,
    dedup_key = core.pr_freshness_dedup_key(REPO, managed_branch, INTEGRATION_SHA),
    source_ref = core.pr_freshness_source_ref(REPO, PR_NUMBER),
    impl_version = version,
  }
end

local function mock_env()
  local values = {
    FKST_GITHUB_WRITE = "1",
    FKST_GITHUB_BOT_LOGIN = BOT,
    FKST_GITHUB_REPO = REPO,
    FKST_GITHUB_CLAIM_MODE = "",
    FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
    FKST_DEVLOOP_INTEGRATION_BRANCH = INTEGRATION_BRANCH,
    FKST_DEVLOOP_MANAGED_BOT_LOGINS = "",
  }
  for name, value in pairs(values) do
    for _ = 1, 64 do
      t.mock_command(devloop_base.read_env_command(name), {
        stdout = value,
        stderr = "",
        exit_code = 0,
      })
    end
  end
end

local function mock_parent_claim()
  entity_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    assignees = { BOT },
    author_login = BOT,
  }, "assignees,author", 30)
end

local function mock_parent_claim_api()
  for _ = 1, 32 do
    t.mock_command("gh api repos/" .. REPO .. "/issues/" .. tostring(ISSUE_NUMBER), {
      stdout = '{"number":' .. tostring(ISSUE_NUMBER)
        .. ',"title":"Abandon exhausted original PR and reimplement"'
        .. ',"body":"Issue body","state":"open"'
        .. ',"created_at":"2026-07-14T01:00:00Z","updated_at":"2026-07-14T01:06:00Z"'
        .. ',"labels":[{"name":"fkst-dev:enabled"},{"name":"fkst-dev:awaiting-pr"}]'
        .. ',"assignees":[{"login":"' .. BOT .. '"}],"user":{"login":"' .. BOT .. '"}}\n',
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_conflict_git(event, final_integration_read)
  local upstream_reads = final_integration_read and 2 or 1
  for _ = 1, upstream_reads do
    t.mock_command("git fetch 'origin' '" .. event.upstream_branch .. "'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("refs/remotes/'origin'/'" .. event.upstream_branch .. "'^{commit}", {
      stdout = event.upstream_sha .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("git fetch 'origin' '" .. event.integration_branch .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("refs/remotes/'origin'/'" .. event.integration_branch .. "'^{commit}", {
    stdout = event.integration_sha .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-integration/2275-acceptance",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --no-ff --no-commit", { stdout = "", stderr = "conflict", exit_code = 1 })
  t.mock_command("ls-files -u", { stdout = UNMERGED, stderr = "", exit_code = 0 })
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_parent_reads(times)
  entity_mocks.mock_issue_read_forms(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Abandon exhausted original PR and reimplement",
    body = "Issue body",
    state = "OPEN",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = parent_comments(ROOT_VERSION),
    assignees = { BOT },
    author_login = BOT,
    times = times,
  })
end

local function mock_freshness_pr(branch, version, state, reads)
  entity_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    head = branch,
    head_sha = PR_HEAD_SHA,
    base_branch = INTEGRATION_BRANCH,
    state = state or "OPEN",
    head_repo = REPO,
    comments = pr_comments("pr-open", version, branch),
    labels = {},
  }, entity_mocks.pr_freshness_selector, reads)
end

local function run_exhausted_conflict(event, opts)
  local options = opts or {}
  local fingerprint = core.sync_conflict_fingerprint(event, UNMERGED)
  core.record_sync_conflict_attempt(event, fingerprint, core.max_sync_conflict_attempts())
  mock_conflict_git(event, options.final_integration_read == true)
  mock_freshness_pr(
    event.integration_branch,
    options.version or ROOT_VERSION,
    options.pr_state or "OPEN",
    options.pr_reads or 1
  )
  if options.parent_reads ~= nil then
    mock_parent_reads(options.parent_reads)
  end
  if options.close == true then
    t.mock_command("gh pr close '" .. tostring(PR_NUMBER) .. "' --repo '" .. REPO .. "'", {
      stdout = "closed\n",
      stderr = "",
      exit_code = 0,
    })
  end
  return graph.run({
    queue = "github-devloop-integration.devloop_sync_conflict",
    payload = event,
    source_ref = delivery_source_ref(event.source_ref),
  }, { max_steps = 8 })
end

local function mock_pr_closed_observation(comments)
  entity_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    head = ORIGINAL_BRANCH,
    head_sha = PR_HEAD_SHA,
    base_branch = INTEGRATION_BRANCH,
    state = "CLOSED",
    head_repo = REPO,
    comments = comments,
    labels = {},
  }, entity_mocks.pr_origin_selector, 1)
  mock_parent_claim()
end

local function mock_terminal_comment_flow()
  local comments_path = "repos/" .. REPO .. "/issues/" .. tostring(PR_NUMBER) .. "/comments?per_page=100"
  for _, command in ipairs({
    "gh api --paginate --slurp " .. comments_path,
    "gh api --paginate --slurp '" .. comments_path .. "'",
  }) do
    t.mock_command(command, {
      stdout = "[[]]\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("gh api --method POST repos/" .. REPO .. "/issues/" .. tostring(PR_NUMBER) .. "/comments --field 'body=", {
    stdout = '{"id":' .. TERMINAL_COMMENT_ID .. ',"body":"created","user":{"login":"' .. BOT .. '"}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh api --method GET 'repos/" .. REPO .. "/issues/comments/" .. TERMINAL_COMMENT_ID .. "'", {
    stdout = '{"body":"' .. h.json_string(core.state_marker(
      PROPOSAL_ID,
      "closed-unmerged",
      ROOT_VERSION
    )) .. '","user":{"login":"' .. BOT .. '"}}\n',
    stderr = "",
    exit_code = 0,
  })
  for _, command in ipairs({
    "gh api --paginate --slurp " .. comments_path,
    "gh api --paginate --slurp '" .. comments_path .. "'",
  }) do
    t.mock_command(command, {
      stdout = '[[{"id":' .. TERMINAL_COMMENT_ID .. ',"body":"'
        .. h.json_string(core.state_marker(PROPOSAL_ID, "closed-unmerged", ROOT_VERSION))
        .. '","user":{"login":"' .. BOT .. '"}}]]\n',
      stderr = "",
      exit_code = 0,
    })
  end
  mock_parent_claim()
  t.mock_command("gh label list", {
    stdout = '[{"name":"fkst-dev:pr-open"},{"name":"fkst-dev:blocked"}]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr edit", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_issue_replay(child_comments, issue_comments, issue_labels)
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  entity_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Abandon exhausted original PR and reimplement",
    body = "Issue body",
    state = "OPEN",
    labels = issue_labels or { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = issue_comments or parent_comments(ROOT_VERSION),
    assignees = { BOT },
    author_login = BOT,
  }, "title,body,comments,labels,state,createdAt,updatedAt,assignees,author", 3)
  entity_mocks.mock_pr_view_selector(t, {
    repo = REPO,
    number = PR_NUMBER,
    head = ORIGINAL_BRANCH,
    head_sha = PR_HEAD_SHA,
    base_branch = INTEGRATION_BRANCH,
    state = "CLOSED",
    head_repo = REPO,
    comments = child_comments,
    labels = {},
  }, entity_mocks.pr_origin_selector, 3)
end

local function issue_observe_event(labels, updated_at)
  local payload_source_ref = entity_lib.issue_source_ref(REPO, ISSUE_NUMBER)
  return {
    queue = "github-devloop.devloop_observe_issue",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = REPO,
      number = ISSUE_NUMBER,
      title = "Abandon exhausted original PR and reimplement",
      state = "OPEN",
      updated_at = updated_at,
      labels = labels,
      dedup_key = REPO .. "#issue#" .. tostring(ISSUE_NUMBER) .. "@" .. updated_at,
      source_ref = payload_source_ref,
    },
    source_ref = delivery_source_ref(payload_source_ref),
  }
end

local function mock_replacement_implementation(ready, ready_body)
  local implementation_branch = devloop_base.implement_branch(
    REPO,
    ISSUE_NUMBER,
    ready.dedup_key
  )
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  entity_mocks.mock_issue_view_selector(t, {
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Abandon exhausted original PR and reimplement",
    body = "Issue body",
    state = "OPEN",
    labels = { "fkst-dev:enabled", "fkst-dev:ready" },
    comments = { comment(ready_body, "2026-07-14T01:07:00Z", READY_COMMENT_ID) },
    assignees = { BOT },
    author_login = BOT,
  }, "title,body,labels,comments,state,author", 5)
  mock_parent_claim()
  for _ = 1, 4 do
    t.mock_command("gh api --method GET 'repos/" .. REPO .. "/issues/comments/" .. READY_COMMENT_ID .. "'", {
      stdout = '{"body":"' .. h.json_string(ready_body)
        .. '","user":{"login":"' .. BOT .. '"}}\n',
      stderr = "",
      exit_code = 0,
    })
  end
  h.mock_context_bundle(ready)
  h.mock_fresh_implement_worktree({
    repo = REPO,
    issue_number = ISSUE_NUMBER,
    impl_version = ready.dedup_key,
  })
  t.mock_command("git fetch 'origin' '" .. INTEGRATION_BRANCH .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("refs/remotes/'origin'/'" .. INTEGRATION_BRANCH .. "'^{commit}", {
    stdout = INTEGRATION_SHA .. "\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("merge --no-edit '" .. INTEGRATION_SHA .. "'", {
    stdout = "Already up to date.\n",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 4 do
    t.mock_command("git show " .. INTEGRATION_SHA .. ":.fkst/substrate-ref", {
      stdout = "3333333333333333333333333333333333333333\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("git show " .. implementation_branch .. ":.fkst/substrate-ref", {
      stdout = "3333333333333333333333333333333333333333\n",
      stderr = "",
      exit_code = 0,
    })
  end
  h.mock_implement_codex(1, "", "acceptance stop after implementation prep")
end

local function mock_ready_and_implementation_outbound()
  local comments_path = "repos/" .. REPO .. "/issues/" .. tostring(ISSUE_NUMBER) .. "/comments?per_page=100"
  for index = 1, 6 do
    for _, command in ipairs({
      "gh api --paginate --slurp " .. comments_path,
      "gh api --paginate --slurp '" .. comments_path .. "'",
    }) do
      t.mock_command(command, { stdout = "[[]]\n", stderr = "", exit_code = 0 })
    end
    local comment_id = tonumber(READY_COMMENT_ID) + index - 1
    t.mock_command("gh api --method POST repos/" .. REPO .. "/issues/" .. tostring(ISSUE_NUMBER) .. "/comments --field 'body=", {
      stdout = '{"id":' .. tostring(comment_id) .. ',"body":"created","user":{"login":"' .. BOT .. '"}}\n',
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 8 do
    t.mock_command("gh label list", {
      stdout = '[{"name":"fkst-dev:ready"},{"name":"fkst-dev:implementing"},{"name":"fkst-dev:impl-failed"}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh issue edit", { stdout = "", stderr = "", exit_code = 0 })
  end
end

local function worktree_add_call()
  for _, call in ipairs(t.command_calls()) do
    if tostring(call.rendered or ""):find("git worktree add -b", 1, true) ~= nil then
      return call
    end
  end
  return nil
end

return {
  test_pr_freshness_abandon_reimplement_acceptance_smoke = function()
    mock_env()
    local original = conflict_event()
    local recovery = run_exhausted_conflict(original, {
      close = true,
      final_integration_read = true,
      parent_reads = 2,
      pr_reads = 2,
    })
    local recovery_step = graph.require_delivery(recovery, {
      queue = "github-devloop-integration.devloop_sync_conflict",
      consumer = "github-devloop-integration.sync_conflict",
    })
    if recovery_step.exit_code ~= 0 then
      error(recovery_step.error, 0)
    end
    t.eq(recovery_step.exit_code, 0)
    t.eq(core.sync_conflict_attempt_count(
      original,
      core.sync_conflict_fingerprint(original, UNMERGED)
    ), core.max_sync_conflict_attempts())
    t.eq(h.count_calls("gh pr close"), 1, "initial exhausted original closes exactly once")
    t.eq(h.count_calls("codex exec"), 0)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
    t.eq(graph.find_raise(recovery, "github-proxy.github_issue_create_request"), nil)
    mock_parent_claim_api()

    local child_comments = pr_comments("pr-open")
    mock_pr_closed_observation(child_comments)
    mock_terminal_comment_flow()
    local observe_source_ref = entity_lib.pr_source_ref(REPO, PR_NUMBER)
    local observe = graph.require_quiescent(graph.run({
      queue = "github-devloop-pr.devloop_observe_pr",
      payload = {
        schema = "github-proxy.v1",
        type = "pr",
        repo = REPO,
        number = PR_NUMBER,
        state = "CLOSED",
        updated_at = "2026-07-14T01:05:00Z",
        dedup_key = REPO .. "#pr#" .. tostring(PR_NUMBER) .. "@2026-07-14T01:05:00Z",
        source_ref = observe_source_ref,
      },
      source_ref = delivery_source_ref(observe_source_ref),
    }, { max_steps = 8 }))
    local terminal = graph.require_raise(observe, "github-proxy.github_pr_comment_request")
    t.eq(terminal.payload.handoff.kind, "github-devloop.closed_unmerged")
    t.is_true(terminal.payload.body:find('state="closed-unmerged"', 1, true) ~= nil)

    graph.require_delivery(observe, {
      queue = "github-proxy.github_comment_written",
      consumer = "github-devloop-pr.comment_handoff",
    })
    table.insert(child_comments, comment(terminal.payload.body, "2026-07-14T01:06:00Z", TERMINAL_COMMENT_ID))
    local child_state = entity_lib.current_entity_state(child_comments, PROPOSAL_ID)
    t.eq(child_state.state, "closed-unmerged")
    t.eq(child_state.version, ROOT_VERSION)

    local ready_marker = core.state_marker(PROPOSAL_ID, "ready", REPLACEMENT_VERSION)
    local expected_ready = payloads_builders.build_devloop_ready_payload(core, {
      proposal_id = PROPOSAL_ID,
      dedup_key = REPLACEMENT_VERSION,
      source_ref = entity_lib.issue_source_ref(REPO, ISSUE_NUMBER),
      include_ready_hand_off = true,
      ready_comment_id = READY_COMMENT_ID,
    })
    mock_issue_replay(child_comments)
    mock_replacement_implementation(expected_ready, ready_marker)
    mock_ready_and_implementation_outbound()
    local awaiting = graph.require_quiescent(graph.run(
      issue_observe_event({ "fkst-dev:enabled", "fkst-dev:awaiting-pr" }, "2026-07-14T01:06:00Z"),
      { max_steps = 30 }
    ))
    local ready_comment = graph.require_raise(awaiting, "github-proxy.github_issue_comment_request", function(raised)
      return tostring(raised.payload and raised.payload.body or ""):find('state="ready"', 1, true) ~= nil
    end)
    t.is_true(ready_comment.payload.body:find("/reimplement/1", 1, true) ~= nil)
    local ready = graph.require_raise(awaiting, "github-devloop.devloop_ready")
    t.eq(ready.payload.ready_hand_off.marker_version, REPLACEMENT_VERSION)
    t.is_true(ready.payload.dedup_key:find("/reimplement/1", 1, true) ~= nil)
    t.eq(ready.payload.dedup_key, expected_ready.dedup_key)

    local implement_step = graph.require_delivery(awaiting, {
      queue = "github-devloop.devloop_ready",
      consumer = "github-devloop.implement",
    })
    t.eq(implement_step.exit_code, 0)
    local add = worktree_add_call()
    local replacement_branch = devloop_base.implement_branch(REPO, ISSUE_NUMBER, ready.payload.dedup_key)
    t.is_true(add ~= nil)
    t.is_true(add.rendered:find(replacement_branch, 1, true) ~= nil)
    t.eq(add.rendered:find(ORIGINAL_BRANCH, 1, true), nil)
    t.is_true(add.rendered:find(INTEGRATION_SHA, 1, true) ~= nil)

    run_exhausted_conflict(original, {
      parent_reads = 1,
      pr_reads = 1,
    })
    t.eq(h.count_calls("gh pr close"), 1, "closed original replay stays idempotent")

    local replacement = conflict_event(REPLACEMENT_BRANCH, REPLACEMENT_VERSION)
    local exhausted_replacement = run_exhausted_conflict(replacement, {
      version = REPLACEMENT_VERSION,
      pr_reads = 1,
    })
    t.eq(h.count_calls("gh pr close"), 1, "replacement exhaustion never closes a second PR")
    t.is_true(graph.find_raise(
      exhausted_replacement,
      "github-proxy.github_issue_create_request"
    ) ~= nil)
  end,
}
