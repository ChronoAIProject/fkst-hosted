local devloop_base = require("devloop.base")
local graph = require("testkit.graph")
local payloads_builders = require("devloop.payloads.builders")
local t = fkst.test
local core = require("core")
local h = require("tests.devloop_base_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function encode_labels_json(labels)
  local rendered = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered, string.format('{"name":"%s"}', h.encode_json_string(label)))
  end
  return table.concat(rendered, ",")
end

local function issue_view_json()
  return string.format(
    '{"title":"%s","body":"%s","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[%s],"comments":[],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    h.encode_json_string("Add retry backoff to failed widget sync"),
    h.encode_json_string("Implement exponential backoff for widget sync retries."),
    encode_labels_json({})
  )
end

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload(core, "owner/repo", 42, "2026-06-03T01:02:03Z")
end

local function initial_event()
  return {
    queue = "github-devloop-intake.devloop_intake_candidate",
    payload = candidate(),
    source_ref = {
      kind = "external",
      reference = "owner/repo#issue/42",
    },
  }
end

local function mock_env()
  for _ = 1, 12 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_CLAIM_MODE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-intake-default-run-graph/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_issue_reads()
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", {
    stdout = issue_view_json(),
  }, 2)
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,updatedAt,labels,comments,state", {
    stdout = string.format(
      '{"title":"%s","body":"%s","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
      h.encode_json_string("Add retry backoff to failed widget sync"),
      h.encode_json_string("Implement exponential backoff for widget sync retries.")
    ),
  })
end

local function mock_context_bundle()
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 3 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
  end
  for _ = 1, 3 do
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-intake-default-run-graph/runtime/context/.bundle-tmp.intake\n",
    stderr = "",
    exit_code = 0,
  })
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,updatedAt,labels,comments,state", {
    stdout = string.format(
      '{"title":"%s","body":"%s","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[],"comments":[]}\n',
      h.encode_json_string("Add retry backoff to failed widget sync"),
      h.encode_json_string("Implement exponential backoff for widget sync retries.")
    ),
  })
  entity_read_mocks.mock_issue_board_digest_list_raw(t, "owner/repo", { stdout = "[]\n" })
  entity_read_mocks.mock_issue_list_raw_command(t, core.gh_issue_list_recent_closed_cmd("owner/repo", 30), { stdout = "[]\n" })
  t.mock_command("gh pr list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  for _ = 1, 8 do
    t.mock_command("touch ", ok)
  end
  for _ = 1, 8 do
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
  end
  t.mock_command("python3 -c", ok)
  for _ = 1, 8 do
    t.mock_command("test -r", ok)
  end
  for _ = 1, 8 do
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
end

local function mock_codex()
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("codex exec", {
    stdout = "⟦FKST:INTAKE⟧ decline\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ run_graph smoke only.",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_run_graph_intake_candidate_delivers_to_default_judge = function()
    mock_env()
    mock_issue_reads()
    mock_context_bundle()
    mock_codex()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "github-devloop-intake.devloop_intake_candidate -> github-devloop-intake-default.intake_judge",
    })

    local step = graph.require_delivery(trace, {
      queue = "github-devloop-intake.devloop_intake_candidate",
      consumer = "github-devloop-intake-default.intake_judge",
    })
    t.eq(step.exit_code, 0)
    t.eq(#step.raises, 2)
    t.eq(step.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.eq(step.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(step.raises[2].payload.add_labels[1], "fkst-class:standard")
    t.eq(step.raises[1].payload.source_ref.ref, source_ref().ref)
  end,
}
