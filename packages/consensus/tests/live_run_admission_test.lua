local identity = require("contract.convergence_identity")
local workflow_codex = require("workflow.codex")
local t = fkst.test
require("tests.cache_seed_helpers")

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/consensus-live-run/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  local root = runtime_root(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = root,
      FKST_RUNTIME_LOG_DIR = root .. "/logs",
    },
  }
end

local function json_string(value)
  return tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function json_value(value)
  if type(value) == "number" then
    return tostring(value)
  end
  if type(value) == "boolean" then
    return value and "true" or "false"
  end
  if value == nil then
    return "null"
  end
  return '"' .. json_string(value) .. '"'
end

local function json_object(record)
  local parts = {}
  for key, value in pairs(record or {}) do
    table.insert(parts, '"' .. json_string(key) .. '":' .. json_value(value))
  end
  table.sort(parts)
  return "{" .. table.concat(parts, ",") .. "}"
end

local function seed_codex_run(run_opts, record)
  local dir = run_opts.env.FKST_RUNTIME_LOG_DIR .. "/codex"
  os.execute("mkdir -p " .. string.format("%q", dir))
  local path = dir .. "/" .. nonce() .. ".log"
  local handle = assert(io.open(path, "a"))
  handle:write("CODEX_STATUS:" .. json_object(record) .. "\n")
  handle:close()
end

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Adopt consensus package",
    body = "Create a small flat package that asks several angles to judge a proposal.",
    content_fetch = "fetch-source --ref demo/consensus/42 --full",
    context = "The package must stay silent unless all angles agree.",
    angles = { "teleology", "parsimony", "fidelity" },
    dedup_key = "proposal-42-v1",
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/42",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function run_decide(event_payload, run_opts)
  return t.run_department("departments/decide/main.lua", {
    queue = "proposal",
    payload = event_payload,
  }, run_opts)
end

local function mock_judgment_runtime()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/consensus-live-run/runtime",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_judgment_dir()
  t.mock_command("mkdir -p", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_angle(angle, verdict, reply)
  mock_judgment_dir()
  t.mock_command("consensus-angle-" .. tostring(angle), {
    stdout = "⟦FKST:VERDICT⟧ " .. verdict .. "\n⟦FKST:REPLY⟧ " .. reply .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

local function with_codex_runs(runs, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = runs or {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
end

return {
  test_convergence_identity_uses_only_stable_proposal_fields = function()
    local built = identity.from_proposal("consensus", proposal({
      dedup_key = "proposal-42-v1/loop/2",
      generation = 7,
      round = 2,
      version = "volatile-version",
      source_ref = { kind = "proposal", ref = "volatile/ref" },
    }), { angle_lane = "teleology" })

    t.eq(built.process.role, "consensus")
    t.eq(built.process.proposal_id, "proposal-42")
    t.eq(built.role, "consensus")
    t.eq(built.proposal_id, "proposal-42")
    t.eq(built.generation, 7)
    t.eq(built.round, 2)
    t.eq(built.angle_lane, "teleology")
    t.eq(built.dedup_key, "convergence:consensus:proposal-42:g7:r2:teleology")
    t.eq(built.version, nil)
    t.eq(built.source_ref, nil)
  end,

  test_convergence_identity_defaults_are_explicit_not_hidden_in_dedup_key = function()
    local built = identity.from_proposal("consensus", proposal(), { angle_lane = "parsimony" })

    t.eq(built.generation, 0)
    t.eq(built.round, 0)
    t.eq(built.angle_lane, "parsimony")
    t.eq(built.dedup_key, "convergence:consensus:proposal-42:g0:r0:parsimony")
  end,

  test_live_run_active_matches_running_identity_only = function()
    local teleology = identity.from_proposal("consensus", proposal(), { angle_lane = "teleology" })
    local parsimony = identity.from_proposal("consensus", proposal(), { angle_lane = "parsimony" })
    with_codex_runs({
      { role = teleology.role, proposal_id = teleology.proposal_id, dedup_key = teleology.dedup_key, status = "running" },
      { role = "consensus", proposal_id = "proposal-99", dedup_key = teleology.dedup_key, status = "running" },
      { role = "fix", proposal_id = "proposal-42", dedup_key = teleology.dedup_key, status = "running" },
      { role = "consensus", proposal_id = "proposal-42", dedup_key = "malformed-missing-status" },
    }, function()
      t.eq(workflow_codex.live_run_active(teleology), true)
      t.eq(workflow_codex.live_run_active(parsimony), false)
      t.eq(workflow_codex.live_run_active("consensus", "proposal-42", "missing"), false)
      t.eq(workflow_codex.live_run_active("fix", "proposal-99", teleology.dedup_key), false)
      t.eq(workflow_codex.live_run_active("consensus", "proposal-42", "malformed-missing-status"), false)
    end)
  end,

  test_workflow_dispatch_sets_identity_fields_and_defers_without_spawn = function()
    local run_identity = identity.from_proposal("consensus", proposal(), { angle_lane = "teleology" })
    with_codex_runs({
      { role = run_identity.role, proposal_id = run_identity.proposal_id, dedup_key = run_identity.dedup_key, status = "running" },
    }, function()
      local result = workflow_codex.dispatch(run_identity, { prompt = "hello", worktree = "/tmp/worktree" })
      t.eq(result.deferred, true)
      t.eq(result.reason, "live-run-active")
      t.eq(#codex_calls(), 0)
    end)
  end,

  test_consensus_decide_dispatches_when_no_live_run_exists = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology approves.")
    mock_angle("parsimony", "approve", "Parsimony approves.")
    mock_angle("fidelity", "approve", "Fidelity approves.")

    with_codex_runs({}, function()
      local result = run_decide(proposal(), opts("no-live-run"))
      t.eq(result.exit_code, 0)
      t.eq(#codex_calls(), 3)
      t.eq(#result.raises, 1)
      t.eq(result.raises[1].queue, "consensus_reached")
    end)
  end,

  test_consensus_decide_defers_when_same_proposal_run_is_live = function()
    mock_judgment_runtime()
    local run_opts = opts("matching-live-run")
    local run_identity = identity.from_proposal("consensus", proposal(), { angle_lane = "teleology" })
    seed_codex_run(run_opts, {
      run_id = nonce(),
      role = run_identity.role,
      proposal_id = run_identity.proposal_id,
      dedup_key = run_identity.dedup_key,
      status = "running",
      started_at = "2026-06-03T00:30:00Z",
      started_at_ms = now() * 1000,
      timeout_seconds = 3600,
      log_path = "/tmp/fkst-packages-test/codex.log",
      cmd_line = "codex exec -",
    })

    local result = run_decide(proposal(), run_opts)
    t.is_true(result.exit_code ~= 0)
    t.eq(#codex_calls(), 0)
    t.eq(#result.raises, 0)
  end,

  test_deferred_delivery_retries_when_live_run_disappears = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology approves.")
    mock_angle("parsimony", "approve", "Parsimony approves.")
    mock_angle("fidelity", "approve", "Fidelity approves.")

    local run_opts = opts("defer-then-redrive")
    local run_identity = identity.from_proposal("consensus", proposal(), { angle_lane = "teleology" })
    seed_codex_run(run_opts, {
      run_id = nonce(),
      role = run_identity.role,
      proposal_id = run_identity.proposal_id,
      dedup_key = run_identity.dedup_key,
      status = "running",
      started_at = "2026-06-03T00:30:00Z",
      started_at_ms = now() * 1000,
      timeout_seconds = 3600,
      log_path = "/tmp/fkst-packages-test/codex.log",
      cmd_line = "codex exec -",
    })
    local deferred = run_decide(proposal(), run_opts)
    t.is_true(deferred.exit_code ~= 0)
    t.eq(#codex_calls(), 0)

    local retried = run_decide(proposal(), opts("redrive-after-live-run-missing"))
    t.eq(retried.exit_code, 0)
    t.eq(#codex_calls(), 3)
    t.eq(#retried.raises, 1)
    t.eq(retried.raises[1].queue, "consensus_reached")
  end,
}
