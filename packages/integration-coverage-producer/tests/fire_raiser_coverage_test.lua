local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

return {
  test_fire_raiser_coverage_poll_routes_and_produces_issue_create_request = function()
    local root = helper.setup_workspace("produce", helper.fire_raiser_child([[
  test_full_produce = function()
    mock_env()
    mock_checker()
    mock_production_issue_reads()

    local trace = t.fire_raiser("coverage_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "integration-coverage-producer.coverage_poll")
    t.eq(trace.routed_to[1], "integration-coverage-producer.produce")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 1)
    t.eq(trace.raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(trace.raised[1].payload.schema, "github-proxy.issue-create.v1")
    t.is_true(trace.raised[1].payload.body:find("coverage-edge-id: autochrono.reply -> github-autochrono.outbound_glue", 1, true) ~= nil)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
