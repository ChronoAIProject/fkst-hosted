local graph = require("testkit.graph")
local t = fkst.test

local repo = "owner/repo"
local issue_number = 1493
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/" .. tostring(issue_number),
  }
end

local function initial_event()
  return {
    queue = "github-proxy.github_issue_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = repo,
      number = issue_number,
      title = "Autochrono bridge",
      url = "https://github.example/owner/repo/issues/" .. tostring(issue_number),
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "owner/repo#issue#1493@2026-06-03T01:02:03Z",
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = "owner/repo#issue/" .. tostring(issue_number),
    },
  }
end

local function mock_downstream_flow()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-autochrono-run-graph/runtime",
    stderr = "",
    exit_code = 0,
  })
  for _, angle in ipairs({ "teleology", "parsimony", "fidelity", "natural-ownership", "proportional-containment" }) do
    t.mock_command("mkdir -p", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("codex exec", {
      stdout = verdict_label .. " approve\n" .. reply_label .. " " .. angle .. " approves.\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_run_graph_issue_changed_delivers_to_inbound_glue = function()
    mock_downstream_flow()
    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 8 }))
    graph.assert_covers(trace, {
      "github-proxy.github_issue_changed -> github-autochrono.inbound_glue",
    })

    local inbound_step, inbound_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_issue_changed",
      consumer = "github-autochrono.inbound_glue",
    })
    t.eq(inbound_step.exit_code, 0)
    t.eq(#(inbound_step.raises or {}), 1)
    t.eq(inbound_index, 1)

    local raised = inbound_step.raises[1]
    t.eq(raised.queue, "autochrono.issue")
    t.eq(raised.payload.schema, "autochrono.issue.v1")
    t.eq(raised.payload.issue_number, issue_number)
    t.eq(raised.payload.source_ref.ref, "owner/repo#issue/" .. tostring(issue_number))
  end,
}
