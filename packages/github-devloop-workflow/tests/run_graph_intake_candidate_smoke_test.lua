local devloop_base = require("devloop.base")
local graph = require("testkit.graph")
local payloads_builders = require("devloop.payloads.builders")
local t = fkst.test
local core = require("core")

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\b", "\\b")
    :gsub("\f", "\\f")
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
    :gsub("[%z\1-\31]", function(char)
      return string.format("\\u%04X", string.byte(char))
    end)
end

local function encode_labels_json(labels)
  local rendered = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered, string.format('{"name":"%s"}', json_string(label)))
  end
  return table.concat(rendered, ",")
end

local function issue_view_json()
  return string.format(
    '{"title":"%s","body":"%s","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[%s],"comments":[],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    json_string("Repair retry backoff for failed widget sync"),
    json_string("Implement exponential backoff for widget sync retries."),
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
  return payloads_builders.build_devloop_intake_candidate_payload("owner/repo", 42, "2026-06-03T01:02:03Z")
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
    t.mock_command(devloop_base.read_env_command("FKST_OUTPUT_LANG"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("printf %s \"$FKST_WORKFLOW_CATALOG_ROOT\"", {
      stdout = "/tmp/fkst-packages-test/github-devloop-workflow-run-graph/no-catalog",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("printf %s \"$FKST_RUNTIME_ROOT\"", {
      stdout = "/tmp/fkst-packages-test/github-devloop-workflow-run-graph/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_issue_reads()
  for _ = 1, 3 do
    t.mock_command("gh issue view", {
      stdout = issue_view_json(),
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_context_bundle()
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 3 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow-run-graph/runtime/context/.bundle-tmp.intake\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue view", {
    stdout = issue_view_json(),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  t.mock_command("gh pr list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  for _ = 1, 8 do
    t.mock_command("touch ", ok)
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
    t.mock_command("test -r", ok)
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("python3 -c", ok)
end

local function mock_workflow_none()
  t.mock_command("mkdir -p", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("codex exec", {
    stdout = "⟦FKST:WORKFLOW_SELECT⟧ none",
    stderr = "",
    exit_code = 0,
  })
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
  test_run_graph_intake_candidate_delivers_to_workflow_select = function()
    mock_env()
    mock_issue_reads()
    mock_workflow_none()
    mock_context_bundle()
    mock_codex()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "github-devloop-intake.devloop_intake_candidate -> github-devloop-workflow.workflow_select",
    })

    local step = graph.require_delivery(trace, {
      queue = "github-devloop-intake.devloop_intake_candidate",
      consumer = "github-devloop-workflow.workflow_select",
    })
    t.eq(step.exit_code, 0)
    t.eq(#step.raises, 2)
    t.eq(step.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.eq(step.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(step.raises[2].payload.add_labels[1], "fkst-class:standard")
    t.eq(step.raises[1].payload.source_ref.ref, source_ref().ref)
  end,
}
