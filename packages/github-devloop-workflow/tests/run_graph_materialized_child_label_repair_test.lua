local base_ids = require("devloop.base_ids")
local core = require("core")
local graph = require("testkit.graph")
local gh_argv = require("testkit.gh_argv_mock")
local t = fkst.test

gh_argv.install(t, core)

local repo = "owner/repo"
local origin_issue = 2200
local child_issue = 2201
local origin = base_ids.proposal_id(repo, origin_issue)
local full_fields = "title,body,updatedAt,labels,comments,state,assignees,author"

local function json_escape(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function comment_json(body)
  return '{"body":"' .. json_escape(body)
    .. '","createdAt":"2026-07-23T00:00:00Z","author":{"login":"fkst-test-bot"}}'
end

local function labels_json(labels)
  local encoded = {}
  for _, label in ipairs(labels or {}) do
    encoded[#encoded + 1] = '{"name":"' .. json_escape(label) .. '"}'
  end
  return table.concat(encoded, ",")
end

local function issue_json(title, body, labels, comments)
  local encoded_comments = {}
  for _, comment in ipairs(comments or {}) do
    encoded_comments[#encoded_comments + 1] = comment_json(comment)
  end
  return '{"title":"' .. json_escape(title)
    .. '","body":"' .. json_escape(body)
    .. '","state":"OPEN","updatedAt":"2026-07-23T00:00:00Z"'
    .. ',"labels":[' .. labels_json(labels) .. ']'
    .. ',"comments":[' .. table.concat(encoded_comments, ",") .. ']'
    .. ',"assignees":[{"login":"fkst-test-bot"}]'
    .. ',"author":{"login":"app/fkst-test-bot"}}\n'
end

local function fixture()
  local blueprint = core.default_catalog.records()[2].blueprint
  local slot = blueprint.steps[1]
  local blueprint_digest = core.digest.blueprint_digest(blueprint)
  local spec = {
    title = slot.title,
    body = "Materialized workflow child fixture.",
  }
  local entry = core.materialization.created_entry(
    origin,
    blueprint_digest,
    slot,
    core.materialization.EMPTY_PREDECESSOR_REF_DIGEST,
    spec,
    child_issue
  )
  local blueprint_marker = core.marker.build_blueprint_marker(origin, blueprint.id, blueprint_digest)
  local ledger_marker = core.marker.build_materialization_marker(
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
  local lineage = core.marker.build_lineage_header(origin, blueprint_digest, slot.id)
  local child_body = lineage .. "\n\n" .. spec.body
    .. "\n\n<!-- fkst:github-proxy:issue-create:" .. entry.child_dedup .. " -->"
  return {
    entry = entry,
    parent = issue_json("Workflow origin", "Run the workflow.", {}, {
      blueprint_marker,
      ledger_marker,
    }),
    child_body = child_body,
    child_title = spec.title,
  }
end

local function mock_env(name, value, times)
  for _ = 1, times or 4 do
    t.mock_command('printf %s "$' .. tostring(name) .. '"', {
      stdout = value or "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_cycle(labels, child_reads)
  local f = fixture()
  mock_env("FKST_GITHUB_REPO", repo)
  mock_env("FKST_GITHUB_BOT_LOGIN", "fkst-test-bot")
  mock_env("FKST_WORKFLOW_CATALOG_ROOT", "")
  mock_env("FKST_SESSION_WORK_LABEL", "fkst-dev,fkst-security")
  mock_env("FKST_GITHUB_CLAIM_MODE", "assignee")
  mock_env("FKST_GITHUB_WRITE", "")

  t.mock_command("gh api --paginate --slurp 'repos/" .. repo .. "/issues?state=open&per_page=100'", {
    stdout = '[[{"number":' .. tostring(origin_issue)
      .. ',"title":"Workflow origin","state":"OPEN","updatedAt":"2026-07-23T00:00:00Z"}]]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue view " .. tostring(origin_issue) .. " --repo " .. repo .. " --json '" .. full_fields .. "'", {
    stdout = f.parent,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue view " .. tostring(origin_issue) .. " --repo " .. repo .. " --json 'assignees,author'", {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"app/fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, child_reads or 1 do
    t.mock_command("gh issue view " .. tostring(child_issue) .. " --repo " .. repo .. " --json '" .. full_fields .. "'", {
      stdout = issue_json(f.child_title, f.child_body, labels, {}),
      stderr = "",
      exit_code = 0,
    })
  end
  return f
end

local function tick(suffix)
  return {
    queue = "github-devloop-workflow.workflow_materialization_tick",
    payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
    source_ref = {
      kind = "cron",
      reference = "github-devloop-workflow.materialization_poll/repair-" .. tostring(suffix),
    },
  }
end

return {
  test_namespaced_repair_replay_routes_add_only_request_and_converges = function()
    mock_cycle({}, 1)
    local first = graph.require_quiescent(graph.run(tick("first"), { max_steps = 4 }))
    graph.assert_covers(first, {
      "github-devloop-workflow.workflow_materialization_tick -> github-devloop-workflow.workflow_materialize_next",
      "github-proxy.github_issue_label_request -> github-proxy.github_issue_label",
    })
    local first_label = graph.require_raise(first, "github-proxy.github_issue_label_request")
    t.eq(first_label.payload.add_labels[1], "fkst-dev")
    t.eq(#first_label.payload.add_labels, 1)
    t.eq(#first_label.payload.remove_labels, 0)
    t.is_nil(first_label.payload.claim)
    t.eq(graph.find_raise(first, "github-proxy.github_issue_create_request"), nil)

    mock_cycle({}, 1)
    local replay = graph.require_quiescent(graph.run(tick("before-visible"), { max_steps = 4 }))
    local replay_label = graph.require_raise(replay, "github-proxy.github_issue_label_request")
    t.eq(replay_label.payload.dedup_key, first_label.payload.dedup_key)
    t.eq(graph.find_raise(replay, "github-proxy.github_issue_create_request"), nil)

    mock_cycle({ "bug", "fkst-dev" }, 2)
    local visible = graph.require_quiescent(graph.run(tick("after-visible"), { max_steps = 4 }))
    graph.assert_covers(visible, {
      "github-devloop-workflow.workflow_materialization_tick -> github-devloop-workflow.workflow_materialize_next",
    })
    t.eq(graph.find_raise(visible, "github-proxy.github_issue_label_request"), nil)
    t.eq(graph.find_raise(visible, "github-proxy.github_issue_create_request"), nil)
    t.eq(graph.find_raise(visible, "github-proxy.github_issue_comment_request"), nil)
  end,
}
