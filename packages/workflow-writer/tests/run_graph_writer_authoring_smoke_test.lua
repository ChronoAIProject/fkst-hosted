-- Integration coverage (run_graph): drive the writer's reconcile department through the
-- REAL composed graph. With no discoverable fkst-workflow request issues (the GitHub
-- port degrades to empty), one materialization tick reaches the materializer and the
-- graph goes quiescent with no downstream raise -- the honest "nothing to author" trace.
--
-- The adapter's productive outbound edge (github-comment-effect.github_issue_comment_request ->
-- github-proxy.github_comment) shares its edge_id with the already-covered github-devloop
-- edge, so the cross-package integration ratchet is satisfied without re-asserting it;
-- this smoke proves the writer's own tick -> materializer wiring drives cleanly.
local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

return {
  test_run_graph_materialization_tick_reaches_materializer_and_quiesces = function()
    local root = helper.setup_workspace("graph", helper.fire_raiser_child([[
  local graph = require("testkit.graph")

  test_writer_materialization_tick_quiesces = function()
    mock_env()

    local trace = graph.require_quiescent(graph.run({
      queue = "workflow-writer.workflow_writer_materialization_tick",
      payload = { schema = "workflow-writer.materialization-tick.v1" },
      source_ref = { kind = "cron", reference = "workflow-writer.writer_poll/materialize" },
    }, { max_steps = 4 }))

    t.eq(trace.routed_to[1], "workflow-writer.workflow_writer_materialize_next")
    t.eq(#trace.raised, 0)
    graph.assert_covers(trace, {})
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
