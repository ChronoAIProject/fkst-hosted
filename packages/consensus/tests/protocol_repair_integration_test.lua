local t = fkst.test

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local stance_label = "⟦FKST:STANCE⟧"

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/consensus-protocol-repair/"
        .. tostring(now()) .. "/" .. nonce() .. "/" .. name,
    },
  }
end

local function proposal(dedup_key)
  return {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-protocol-repair",
    title = "Repair a malformed consensus response",
    body = "Judge a bounded proposal and preserve fail-closed protocol behavior.",
    content_fetch = "fetch-source --ref demo/consensus/protocol-repair --full",
    context = "One successful parser repair must remain inside the current delivery.",
    angles = { "teleology", "parsimony", "fidelity" },
    dedup_key = dedup_key,
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/protocol-repair",
    },
  }
end

local function run_decide(value, run_opts)
  return t.run_department("departments/decide/main.lua", {
    queue = "proposal",
    payload = value,
  }, run_opts)
end

local function mock_judgment_runtime()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/consensus-protocol-repair/runtime",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_judgment_dir()
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_angle(angle, verdict, reply)
  mock_judgment_dir()
  t.mock_command("consensus-angle-" .. tostring(angle), {
    stdout = verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_angle_repair(angle, verdict, reply)
  mock_judgment_dir()
  t.mock_command("consensus-repair-blind-" .. tostring(angle), {
    stdout = verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_rebuttal(angle, verdict, reply)
  mock_judgment_dir()
  t.mock_command("consensus-rebuttal-" .. tostring(angle), {
    stdout = stance_label .. " defend\n"
      .. verdict_label .. " " .. verdict .. "\n"
      .. reply_label .. " " .. reply .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_rebuttal_repair(angle, verdict, reply)
  mock_judgment_dir()
  t.mock_command("consensus-repair-rebuttal-" .. tostring(angle), {
    stdout = stance_label .. " defend\n"
      .. verdict_label .. " " .. verdict .. "\n"
      .. reply_label .. " " .. reply .. "\n",
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

local function judgment_call(role)
  for _, call in ipairs(codex_calls()) do
    if call.rendered:find("/judgment-worktrees/consensus-" .. role, 1, true) ~= nil then
      return call
    end
  end
  return nil
end

local function assert_judgment_worktree(call, role)
  t.is_true(call.rendered:find(" -C ", 1, true) ~= nil)
  t.is_true(call.rendered:find("/judgment-worktrees/consensus-" .. role, 1, true) ~= nil)
end

return {
  test_malformed_blind_answer_repairs_inside_one_delivery_and_reaches_consensus = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", {
      stdout = verdict_label .. " approve\nnot a reply marker\n",
      exit_code = 0,
    })
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_angle_repair("parsimony", "approve", "Parsimony emits the corrected contract.")

    local result = run_decide(
      proposal("proposal-protocol-repair/blind-success"),
      opts("blind-success")
    )

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(#codex_calls(), 4)

    local repair = judgment_call("repair-blind-parsimony")
    assert_judgment_worktree(repair, "repair-blind-parsimony")
    t.is_true(repair.stdin:find("Repair attempt for Phase B (blind):", 1, true) ~= nil)
    t.is_true(repair.stdin:find("class=reply_marker_missing", 1, true) ~= nil)
    t.is_true(repair.stdin:find("> > " .. verdict_label .. " approve", 1, true) ~= nil)
  end,

  test_blind_repair_exhaustion_fails_closed_with_typed_first_and_final_violations = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", {
      stdout = verdict_label .. " approve\nnot a reply marker\n",
      exit_code = 0,
    })
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-repair-blind-parsimony", { stdout = "still malformed", exit_code = 0 })

    local result = run_decide(
      proposal("proposal-protocol-repair/blind-exhausted"),
      opts("blind-exhausted")
    )

    t.is_true(result.exit_code ~= 0)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.error):find("angle-output-unparseable", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("phase=blind", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("repair_attempts=1", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("protocol_violations=verdict_marker_missing:1", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("first_protocol_violations=reply_marker_missing:1", 1, true) ~= nil)
    t.eq(#codex_calls(), 4)
  end,

  test_blind_repair_worker_failure_is_not_reinterpreted_or_repaired_again = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", { stdout = "malformed", exit_code = 0 })
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-repair-blind-parsimony", {
      stderr = "repair worker failed",
      exit_code = 7,
    })

    local result = run_decide(
      proposal("proposal-protocol-repair/blind-worker-failed"),
      opts("blind-worker-failed")
    )

    t.is_true(result.exit_code ~= 0)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.error):find("codex-failed", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("phase=blind", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("repair_attempts=1", 1, true) ~= nil)
    t.is_true(tostring(result.error):find("repair worker failed", 1, true) ~= nil)
    t.eq(#codex_calls(), 4)
  end,

  test_malformed_rebuttal_repairs_once_and_reaches_post_rebuttal_consensus = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle initially abstains.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal("teleology", "approve", "Teleology still approves.")
    mock_judgment_dir()
    t.mock_command("consensus-rebuttal-parsimony", {
      stdout = stance_label .. " defend\n" .. verdict_label .. " approve\nnot a reply marker\n",
      exit_code = 0,
    })
    mock_rebuttal("fidelity", "approve", "Fidelity still approves.")
    mock_rebuttal_repair("parsimony", "approve", "Parsimony emits the corrected rebuttal.")

    local result = run_decide(
      proposal("proposal-protocol-repair/rebuttal-success"),
      opts("rebuttal-success")
    )

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(result.raises[1].payload.verdict_path, "post-rebuttal-unanimity")
    t.eq(#codex_calls(), 7)
    t.eq(judgment_call("synthesis"), nil)

    local repair = judgment_call("repair-rebuttal-parsimony")
    assert_judgment_worktree(repair, "repair-rebuttal-parsimony")
    t.is_true(repair.stdin:find("Repair attempt for Phase R (rebuttal):", 1, true) ~= nil)
    t.is_true(repair.stdin:find("class=reply_marker_missing", 1, true) ~= nil)
    t.is_true(repair.stdin:find("> > " .. stance_label .. " defend", 1, true) ~= nil)
  end,
}
