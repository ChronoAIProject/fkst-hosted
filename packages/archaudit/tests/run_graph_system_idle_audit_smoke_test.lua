local graph = require("testkit.graph")
local t = fkst.test

local function observe_facts()
  return {
    schema_version = 1,
    generated_at_ms = 1781830860000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 500, max_dead_letters = 500 },
    truncated = { deliveries = false, dead_letters = false },
    queues = {
      { queue = "proposal", depth = 0, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil },
    },
    deliveries = json.decode("[]"),
    dead_letters = json.decode("[]"),
  }
end

local function initial_event()
  local detected_at = "2026-06-19T01:00:00Z"
  return {
    queue = "idle-detector.system_idle",
    payload = {
      schema = "idle-detector.system-idle.v1",
      detected_at = detected_at,
      expires_at = "2026-06-19T01:10:00Z",
      source_ref = { kind = "host-observe", ref = "idle_tick/" .. detected_at },
    },
    source_ref = { kind = "cron", reference = "idle-detector/idle_poll/" .. detected_at },
  }
end

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = "3", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_github()
  t.mock_command("gh issue list --repo owner/repo --state all --limit 100 --search fkst:archaudit:audit-run:v1 --json 'number,title,state,author,body,url,createdAt,updatedAt'", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh label list --repo owner/repo --limit 1000 --json name", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_audit_codex()
  t.mock_command("codex exec", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_run_graph_system_idle_delivers_to_archaudit_audit = function()
    t.mock_observe(observe_facts())
    mock_env()
    mock_github()
    mock_audit_codex()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "idle-detector.system_idle -> archaudit.audit",
    })
  end,
}
