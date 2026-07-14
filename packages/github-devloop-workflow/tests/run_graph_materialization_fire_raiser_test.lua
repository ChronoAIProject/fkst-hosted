local devloop_base = require("devloop.base")
local t = fkst.test
local core = require("core")
local graph = require("testkit.graph")
local gh_argv = require("testkit.gh_argv_mock")
local base_ids = require("devloop.base_ids")
local m_builders = require("devloop.markers.builders")
local github_commands = require("forge.github").new(function() end)
gh_argv.install(t, core)

local repo = "owner/repo"
local origin_issue = 2133
local first_child_issue = 2134
local revived_child_issue = 2137
local origin = base_ids.proposal_id(repo, origin_issue)
local first_child = base_ids.proposal_id(repo, first_child_issue)
local revived_child = base_ids.proposal_id(repo, revived_child_issue)
local first_pr = 2135
local revived_pr = 2139
local child_version = "ready/consensus-workflow-child/2026-07-10T20-18-00Z"
local head_sha = "0123456789abcdef0123456789abcdef01234567"
local merge_commit_sha = "1111111111111111111111111111111111111111"
local rollup_head_sha = "2222222222222222222222222222222222222222"
local rollup_pr = 2140
local integration_branch = "integration-elonsg"
local upstream_branch = "dev"
local revived_branch = "devloop-owner-repo-2137-01HY"
local blocked_child_version = child_version .. "/blocked/child-pr-blocked"

local function json_escape(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function comment_json(body, created_at)
  return string.format(
    '{"body":"%s","createdAt":"%s","author":{"login":"fkst-test-bot"}}',
    json_escape(body),
    tostring(created_at or "2026-07-10T20:18:00Z")
  )
end

local function issue_json(number, title, labels, comments, state)
  local comment_parts = {}
  for index, item in ipairs(comments or {}) do
    comment_parts[index] = comment_json(item.body or item, item.created_at)
  end
  local label_parts = {}
  for index, label in ipairs(labels or {}) do
    label_parts[index] = string.format('{"name":"%s"}', json_escape(label))
  end
  return string.format(
    '{"number":%d,"title":"%s","body":"fixture","state":"%s","createdAt":"2026-07-10T20:00:00Z","updatedAt":"2026-07-12T00:25:02Z","labels":[%s],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    number,
    json_escape(title),
    tostring(state or "OPEN"),
    table.concat(label_parts, ","),
    table.concat(comment_parts, ",")
  )
end

local function rest_comments_json(comments)
  local parts = {}
  for index, item in ipairs(comments or {}) do
    parts[index] = string.format(
      '{"id":%d,"body":"%s","user":{"login":"fkst-test-bot"},"created_at":"%s"}',
      index,
      json_escape(item.body or item),
      tostring(item.created_at or "2026-07-10T20:18:00Z")
    )
  end
  return "[[" .. table.concat(parts, ",") .. "]]\n"
end

local function ownership_json()
  return '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n'
end

local function created_materialization_marker(blueprint, slot, predecessor_digest, child_issue)
  local spec = {
    title = slot.title,
    body = "Materialized workflow child fixture.",
  }
  local entry = core.materialization.write_generated_entry(
    origin,
    core.digest.blueprint_digest(blueprint),
    slot,
    predecessor_digest,
    spec
  )
  local built, err = core.marker.build_materialization_marker(
    origin,
    entry.blueprint_digest,
    entry.slot,
    entry.predecessor_ref_digest,
    entry.gen_contract_digest,
    entry.gen_spec_digest,
    entry.child_dedup,
    tostring(child_issue),
    "created"
  )
  t.is_nil(err)
  return built
end

local function workflow_history(include_revived_child, terminal_body)
  local blueprint = core.default_catalog.records()[2].blueprint
  local blueprint_digest = core.digest.blueprint_digest(blueprint)
  local blueprint_marker, blueprint_err = core.marker.build_blueprint_marker(origin, blueprint.id, blueprint_digest)
  t.is_nil(blueprint_err)
  local first_predecessor = core.materialization.EMPTY_PREDECESSOR_REF_DIGEST
  local second_predecessor = core.materialize_reconcile._private.predecessor_ref_digest({
    proposal_id = first_child,
    source_ref = { kind = "external", ref = repo .. "#issue/" .. tostring(first_child_issue) },
  })
  local comments = {
    { body = blueprint_marker },
    { body = created_materialization_marker(blueprint, blueprint.steps[1], first_predecessor, first_child_issue) },
  }
  if include_revived_child then
    comments[#comments + 1] = {
      body = created_materialization_marker(blueprint, blueprint.steps[2], second_predecessor, revived_child_issue),
    }
  end
  if terminal_body ~= nil then
    comments[#comments + 1] = { body = terminal_body, created_at = "2026-07-10T20:43:00Z" }
  end
  return comments
end

local function child_history(proposal_id, issue_number, pr_number, merged)
  local body = ""
  if merged then
    body = m_builders.pr_delegation_marker(
      proposal_id,
      "github-devloop/pr/" .. repo .. "/" .. tostring(pr_number),
      pr_number,
      child_version,
      "g1"
    ) .. "\n" .. m_builders.merged_marker(core, proposal_id, pr_number, child_version, head_sha)
  end
  return issue_json(
    issue_number,
    "Workflow child",
    { merged and "fkst-dev:merged" or "fkst-dev:blocked" },
    { { body = body } }
  )
end

local function revived_child_body()
  return core.state_marker(revived_child, "blocked", blocked_child_version)
    .. "\n" .. m_builders.pr_delegation_marker(
      revived_child,
      "github-devloop/pr/" .. repo .. "/" .. tostring(revived_pr),
      revived_pr,
      child_version,
      "g1"
    )
end

local function revived_child_history(state)
  return issue_json(
    revived_child_issue,
    "Workflow child",
    { "fkst-dev:enabled", "fkst-dev:blocked" },
    { { body = revived_child_body() } },
    state
  )
end

local function pr_origin_body()
  return m_builders.pr_origin_marker(
    revived_child,
    revived_child_issue,
    revived_branch,
    child_version,
    integration_branch
  )
end

local function pr_view_json(state)
  local merged_at = state == "MERGED" and "2026-07-12T00:25:02Z" or ""
  return string.format(
    '{"number":%d,"title":"Workflow child PR","body":"fixture","headRefName":"%s","headRefOid":"%s","baseRefName":"%s","state":"%s","updatedAt":"2026-07-12T00:25:02Z","mergedAt":"%s","comments":[%s],"labels":[],"author":{"login":"fkst-test-bot"},"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN"}\n',
    revived_pr,
    revived_branch,
    head_sha,
    integration_branch,
    state,
    merged_at,
    comment_json(pr_origin_body(), "2026-07-10T20:20:00Z")
  )
end

local function pr_rest_json()
  return string.format(
    '{"number":%d,"state":"closed","updated_at":"2026-07-12T00:25:02Z","merged_at":"2026-07-12T00:25:02Z","merge_commit_sha":"%s","draft":false,"labels":[],"user":{"login":"fkst-test-bot"},"mergeable":true,"mergeable_state":"clean","head":{"ref":"%s","sha":"%s","repo":{"full_name":"%s","owner":{"login":"owner"}}},"base":{"ref":"%s","sha":"abc123","repo":{"full_name":"%s","owner":{"login":"owner"}}}}\n',
    revived_pr,
    merge_commit_sha,
    revived_branch,
    head_sha,
    repo,
    integration_branch,
    repo
  )
end

local function issue_rest_json()
  return string.format(
    '{"number":%d,"title":"Workflow child","body":"fixture","state":"open","created_at":"2026-07-10T20:00:00Z","updated_at":"2026-07-12T00:25:02Z","labels":[{"name":"fkst-dev:enabled"},{"name":"fkst-dev:blocked"}],"user":{"login":"fkst-test-bot"},"assignees":[{"login":"fkst-test-bot"}]}\n',
    revived_child_issue
  )
end

local function mock_pr_view(state)
  t.mock_command(core.gh_pr_view_origin_cmd(repo, revived_pr), {
    stdout = pr_view_json(state), stderr = "", exit_code = 0,
  })
end

local function mock_materialization_cycle(origin_comments, revived_state, pr_state, releases_claim)
  t.mock_command("gh api --paginate --slurp 'repos/" .. repo .. "/issues?state=open&per_page=100'", {
    stdout = '[[{"number":' .. tostring(origin_issue) .. ',"title":"Workflow origin","state":"OPEN","updatedAt":"2026-07-12T00:25:02Z"}]]\n',
    stderr = "",
    exit_code = 0,
  })
  local full_fields = "title,body,updatedAt,labels,comments,state,assignees,author"
  t.mock_command("gh issue view " .. tostring(origin_issue) .. " --repo " .. repo .. " --json '" .. full_fields .. "'", {
    stdout = issue_json(origin_issue, "Workflow origin", {}, origin_comments), stderr = "", exit_code = 0,
  })
  t.mock_command("gh issue view " .. tostring(origin_issue) .. " --repo " .. repo .. " --json 'assignees,author'", {
    stdout = ownership_json(), stderr = "", exit_code = 0,
  })
  t.mock_command("gh issue view " .. tostring(first_child_issue) .. " --repo " .. repo .. " --json '" .. full_fields .. "'", {
    stdout = child_history(first_child, first_child_issue, first_pr, true), stderr = "", exit_code = 0,
  })
  if revived_state ~= nil then
    t.mock_command("gh issue view " .. tostring(revived_child_issue) .. " --repo " .. repo .. " --json '" .. full_fields .. "'", {
      stdout = revived_child_history(revived_state), stderr = "", exit_code = 0,
    })
    mock_pr_view(pr_state or "OPEN")
  end
  if releases_claim then
    t.mock_command("gh issue view " .. tostring(origin_issue) .. " --repo " .. repo .. " --json 'assignees,author'", {
      stdout = ownership_json(), stderr = "", exit_code = 0,
    })
  end
end

local function mock_env()
  for _ = 1, 9 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
      stdout = repo,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_WORKFLOW_CATALOG_ROOT"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_write_mode(value, times)
  for _ = 1, times or 1 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = value or "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_child_materialization()
  for _ = 1, 2 do
    t.mock_command("gh issue list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("codex exec", {
    stdout = '{"title":"Workflow child","body":"Materialized workflow child fixture."}',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_native_merge_observation()
  mock_write_mode("1", 4)
  t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
    stdout = upstream_branch, stderr = "", exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
    stdout = integration_branch, stderr = "", exit_code = 0,
  })
  t.mock_command("gh api 'repos/" .. repo .. "/pulls/" .. tostring(revived_pr) .. "'", {
    stdout = pr_rest_json(), stderr = "", exit_code = 0,
  })
  t.mock_command("gh api --paginate --slurp 'repos/" .. repo .. "/issues/" .. tostring(revived_pr) .. "/comments?per_page=100'", {
    stdout = rest_comments_json({ { body = pr_origin_body() } }), stderr = "", exit_code = 0,
  })
  t.mock_command("gh api 'repos/" .. repo .. "/issues/" .. tostring(revived_child_issue) .. "'", {
    stdout = issue_rest_json(), stderr = "", exit_code = 0,
  })
  t.mock_command("gh api --paginate --slurp 'repos/" .. repo .. "/issues/" .. tostring(revived_child_issue) .. "/comments?per_page=100'", {
    stdout = rest_comments_json({ { body = revived_child_body() } }),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(github_commands.pr_list_promotions_cmd(repo, integration_branch, upstream_branch), {
    stdout = '[[{"number":' .. tostring(rollup_pr)
      .. ',"state":"closed","merged_at":"2026-07-12T00:24:02Z"'
      .. ',"head":{"ref":"' .. integration_branch .. '","sha":"' .. rollup_head_sha
      .. '","repo":{"full_name":"' .. repo .. '"}},"base":{"ref":"' .. upstream_branch .. '"}}]]\n',
    stderr = "", exit_code = 0,
  })
  t.mock_command(core.git_fetch_pr_head_ref_cmd("origin", rollup_pr), {
    stdout = "", stderr = "", exit_code = 0,
  })
  t.mock_command(core.git_fetch_head_commit_cmd(), {
    stdout = rollup_head_sha .. "\n", stderr = "", exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor " .. merge_commit_sha .. " " .. rollup_head_sha, {
    stdout = "", stderr = "", exit_code = 0,
  })
  t.mock_command(core.gh_issue_close_cmd(repo, revived_child_issue), {
    stdout = "closed\n", stderr = "", exit_code = 0,
  })
end

local function native_pr_merged_event()
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = revived_pr,
      state = "MERGED",
      updated_at = "2026-07-12T00:25:02Z",
      dedup_key = repo .. "#pr#" .. tostring(revived_pr) .. "@2026-07-12T00:25:02Z",
      source_ref = { kind = "external", ref = repo .. "#pr/" .. tostring(revived_pr) },
    },
    source_ref = { kind = "external", reference = repo .. "#pr/" .. tostring(revived_pr) },
  }
end

local function mock_empty_origin_list()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_fire_raiser_materialization_poll_routes_real_tick_to_materializer = function()
    mock_env()
    mock_empty_origin_list()
    local trace = t.fire_raiser("materialization_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "github-devloop-workflow.materialization_poll")
    t.eq(trace.routed_to[1], "github-devloop-workflow.workflow_materialize_next")
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
    graph.assert_covers(trace, {})
  end,

  test_run_graph_rederives_revived_merged_child_after_child_fatal = function()
    mock_env()
    mock_write_mode("", 4)
    mock_child_materialization()
    mock_materialization_cycle(workflow_history(false), nil, nil, false)

    local materialized_trace = graph.require_quiescent(graph.run({
      queue = "github-devloop-workflow.workflow_materialization_tick",
      payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
      source_ref = { kind = "cron", reference = "github-devloop-workflow.materialization_poll/materialize" },
    }, { max_steps = 4 }))
    graph.assert_covers(materialized_trace, {
      "github-devloop-workflow.workflow_materialization_tick -> github-devloop-workflow.workflow_materialize_next",
      "github-proxy.github_issue_create_request -> github-proxy.github_issue_create",
    })
    local create = graph.require_raise(materialized_trace, "github-proxy.github_issue_create_request")
    t.eq(create.payload.title, "Workflow child")
    t.is_true(create.payload.body:find("Materialized workflow child fixture.", 1, true) ~= nil)

    mock_write_mode("", 4)
    mock_materialization_cycle(workflow_history(true), "OPEN", "OPEN", false)

    local fatal_trace = graph.require_quiescent(graph.run({
      queue = "github-devloop-workflow.workflow_materialization_tick",
      payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
      source_ref = { kind = "cron", reference = "github-devloop-workflow.materialization_poll/fatal" },
    }, { max_steps = 4 }))
    local fatal = graph.find_raise(fatal_trace, "github-proxy.github_issue_comment_request")
    if fatal == nil then
      local calls = {}
      for index, call in ipairs(t.command_calls()) do
        calls[index] = tostring(call.rendered or call.command or call.cmd or call)
      end
      local step = fatal_trace.steps and fatal_trace.steps[1] or {}
      error("fatal replay produced no terminal comment; stdout=" .. tostring(step.stdout)
        .. " stderr=" .. tostring(step.stderr)
        .. " commands=" .. table.concat(calls, " | "))
    end
    t.is_true(fatal.payload.body:find('state="blocked"', 1, true) ~= nil)
    t.is_true(fatal.payload.body:find('reason_code="child-fatal-behavior-preserving-restructure"', 1, true) ~= nil)

    mock_native_merge_observation()
    local merged_trace = graph.require_quiescent(graph.run(native_pr_merged_event(), { max_steps = 4 }))
    graph.assert_covers(merged_trace, {
      "github-proxy.github_entity_changed -> github-devloop.observe_issue",
    })
    local close_calls = 0
    for _, call in ipairs(t.command_calls()) do
      if gh_argv.call_contains(call, "gh issue close " .. tostring(revived_child_issue))
        and gh_argv.call_contains(call, "--repo " .. repo) then
        close_calls = close_calls + 1
      end
    end
    t.eq(close_calls, 1)

    mock_write_mode("", 6)
    mock_materialization_cycle(workflow_history(true, fatal.payload.body), "CLOSED", "MERGED", true)
    local recovered_trace = graph.require_quiescent(graph.run({
      queue = "github-devloop-workflow.workflow_materialization_tick",
      payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
      source_ref = { kind = "cron", reference = "github-devloop-workflow.materialization_poll/recovered" },
    }, { max_steps = 4 }))
    graph.assert_covers(recovered_trace, {
      "github-devloop-workflow.workflow_materialization_tick -> github-devloop-workflow.workflow_materialize_next",
    })
    local done = graph.require_raise(recovered_trace, "github-proxy.github_issue_comment_request")
    t.is_true(done.payload.body:find('state="done"', 1, true) ~= nil)
    t.is_true(done.payload.body:find('reason_code="all-slots-result-ready"', 1, true) ~= nil)
  end,
}
