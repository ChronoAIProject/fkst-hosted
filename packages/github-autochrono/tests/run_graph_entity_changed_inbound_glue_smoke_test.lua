local graph = require("testkit.graph")
local t = fkst.test

local repo = "owner/repo"
local pr_number = 1493

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#pr/" .. tostring(pr_number),
  }
end

local function initial_event()
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = pr_number,
      title = "Autochrono bridge",
      url = "https://github.example/owner/repo/pull/" .. tostring(pr_number),
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "owner/repo#pr/1493@2026-06-03T01:02:03Z",
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = "owner/repo#pr/" .. tostring(pr_number),
    },
  }
end

return {
  test_run_graph_entity_changed_delivers_to_inbound_glue = function()
    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 2 }))
    graph.assert_covers(trace, {
      "github-proxy.github_entity_changed -> github-autochrono.inbound_glue",
    })

    local inbound_step, inbound_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_entity_changed",
      consumer = "github-autochrono.inbound_glue",
    })
    t.eq(inbound_step.exit_code, 0)
    t.eq(#(inbound_step.raises or {}), 0)
    t.eq(inbound_index, 1)
  end,
}
