local t = fkst.test
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/consensus/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = runtime_root(name),
    },
  }
end

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Adopt consensus package",
    body = "Create a small flat package that asks several angles to judge a proposal.",
    content_fetch = "fetch-source --ref demo/consensus/42 --full",
    context = "The package must stay silent unless all angles agree.",
    angles = { "minimal", "structural", "delete" },
    dedup_key = "proposal-42-v1",
    -- Source-agnostic sample: an opaque {kind, ref} pointer, not tied to any provider.
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

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

local function has_stdin_fragment(calls, fragment)
  for _, call in ipairs(calls) do
    if tostring(call.stdin or ""):find(fragment, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function count_digest_verdicts(digests, verdict)
  local count = 0
  for _, digest in ipairs(digests or {}) do
    if digest.verdict == verdict then
      count = count + 1
    end
  end
  return count
end

local function mock_angle(verdict, reply, exit_code)
  t.mock_command("codex exec", {
    stdout = verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function mock_meta(line, exit_code)
  t.mock_command("codex exec", {
    stdout = tostring(line or "") .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

return {
  test_all_angles_approve_raises_consensus_reached = function()
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("approve", "Delete angle approves.")

    local result = run_decide(proposal(), opts("all-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.schema, "consensus.consensus_reached.v1")
    t.eq(result.raises[1].payload.proposal_id, "proposal-42")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(result.raises[1].payload.dedup_key, "consensus:proposal-42-v1")
    t.eq(result.raises[1].payload.source_ref.kind, "proposal")
    t.eq(result.raises[1].payload.source_ref.ref, "demo/consensus/42")
    t.eq(#result.raises[1].payload.angle_results, 3)
    t.eq(result.raises[1].payload.angle_results[1].angle, "minimal")
    t.eq(result.raises[1].payload.angle_results[2].angle, "structural")
    t.eq(result.raises[1].payload.angle_results[3].angle, "delete")

    local calls = codex_calls()
    t.eq(#calls, 3)
    t.is_true(has_stdin_fragment(calls, "Angle: minimal"))
    t.is_true(has_stdin_fragment(calls, "Angle: structural"))
    t.is_true(has_stdin_fragment(calls, "Angle: delete"))
    t.is_true(has_stdin_fragment(calls, "source_ref.ref: demo/consensus/42"))
    t.is_true(has_stdin_fragment(calls, "fetch-source --ref demo/consensus/42 --full"))
  end,

  test_codex_stdin_carries_fetch_instruction_not_full_body = function()
    local full_tail = "FULL_BODY_TAIL_MUST_NOT_REACH_CODEX"
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("approve", "Delete angle approves.")

    local result = run_decide(proposal({
      body = "Brief only.",
      content_fetch = "fetch-source --ref demo/consensus/42 --full",
      context = nil,
      full_body = string.rep("x", 16000) .. full_tail,
    }), opts("stdin-fetch-not-full-body"))

    t.eq(result.exit_code, 0)
    local calls = codex_calls()
    t.eq(#calls, 3)
    t.is_true(calls[1].stdin:find("Brief only.", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("fetch-source --ref demo/consensus/42 --full", 1, true) ~= nil)
    t.is_nil(calls[1].stdin:find(full_tail, 1, true))
  end,

  test_unanimous_abstain_raises_consensus_converge = function()
    mock_angle("abstain", "Minimal angle needs narrower scope.")
    mock_angle("abstain", "Structural angle needs clearer boundaries.")
    mock_angle("abstain", "Delete angle needs proof the scope is necessary.")
    mock_meta("converge: What concrete evidence would make the narrowed scope approvable?")

    local result = run_decide(proposal(), opts("all-abstain"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(result.raises[1].payload.narrowed_question, "What concrete evidence would make the narrowed scope approvable?")
    t.eq(#codex_calls(), 4)
  end,

  test_split_verdicts_spawn_meta_and_raise_consensus_converge = function()
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("abstain", "Structural angle needs one blocker resolved.")
    mock_angle("approve", "Delete angle approves.")
    mock_meta("converge: Should structural concerns block this proposal?")

    local result = run_decide(proposal(), opts("split"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(result.raises[1].payload.schema, "consensus.consensus_converge.v1")
    t.eq(result.raises[1].payload.proposal_id, "proposal-42")
    t.eq(result.raises[1].payload.dedup_key, "consensus:proposal-42-v1")
    t.eq(result.raises[1].payload.round, 0)
    t.eq(result.raises[1].payload.narrowed_question, "Should structural concerns block this proposal?")
    t.eq(result.raises[1].payload.source_ref.kind, "proposal")
    t.eq(result.raises[1].payload.source_ref.ref, "demo/consensus/42")
    t.eq(#result.raises[1].payload.angle_digests, 3)
    t.eq(count_digest_verdicts(result.raises[1].payload.angle_digests, "approve"), 2)
    t.eq(count_digest_verdicts(result.raises[1].payload.angle_digests, "abstain"), 1)
    t.is_nil(result.raises[1].payload.body)
    t.is_nil(result.raises[1].payload.angle_results)
    t.is_nil(result.raises[1].payload.decision)
    local calls = codex_calls()
    t.eq(#calls, 4)
    t.is_true(calls[4].stdin:find("Angle outputs:", 1, true) ~= nil)
  end,

  test_converge_mode_reject_outputs_raise_consensus_converge = function()
    mock_angle("reject", "Minimal angle rejects but converge mode cannot reject.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("approve", "Delete angle approves.")
    mock_meta("converge: What concern prevents approval?")

    local result = run_decide(proposal({ verdict_mode = "converge" }), opts("converge-reject-output"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(count_digest_verdicts(result.raises[1].payload.angle_digests, "invalid"), 1)
    t.eq(result.raises[1].payload.narrowed_question, "What concern prevents approval?")
    t.eq(#codex_calls(), 4)
  end,

  test_gate_mode_unanimous_reject_raises_consensus_reached_reject = function()
    mock_angle("reject", "Minimal angle rejects the diff.")
    mock_angle("reject", "Structural angle rejects the diff.")
    mock_angle("reject", "Delete angle rejects the diff.")

    local result = run_decide(proposal({ verdict_mode = "gate" }), opts("gate-all-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "reject")
    t.eq(#codex_calls(), 3)
  end,

  test_gate_mode_meta_reject_raises_consensus_reached_reject = function()
    mock_angle("reject", "Minimal angle rejects the diff.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("reject", "Delete angle rejects the diff.")
    mock_meta("reached:reject reject until the failing test is fixed")

    local result = run_decide(proposal({ verdict_mode = "gate" }), opts("gate-meta-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "reject")
    t.eq(result.raises[1].payload.framing, "reject reject until the failing test is fixed")
    t.eq(result.raises[1].payload.body:find("Meta-judge framing:", 1, true), nil)
    t.eq(#codex_calls(), 4)
  end,

  test_meta_reached_after_split_raises_consensus_reached = function()
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("abstain", "Structural angle abstains but accepts the narrowed framing.")
    mock_angle("approve", "Delete angle approves.")
    mock_meta("reached:approve approve the narrowed framing")

    local result = run_decide(proposal(), opts("split-meta-reached"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.schema, "consensus.consensus_reached.v1")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(result.raises[1].payload.framing, "approve approve the narrowed framing")
    t.eq(result.raises[1].payload.body:find("Meta-judge framing:", 1, true), nil)
    t.eq(#codex_calls(), 4)
  end,

  test_abstain_raises_consensus_converge = function()
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("abstain", "Structural angle abstains.")
    mock_angle("approve", "Delete angle approves.")
    mock_meta("converge: Ask structural to name the one blocker that prevents approval.")

    local result = run_decide(proposal(), opts("abstain"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(#codex_calls(), 4)
  end,

  test_failed_codex_call_raises_consensus_converge = function()
    mock_angle("approve", "Minimal angle approves.")
    t.mock_command("codex exec", {
      stderr = "forced failure",
      exit_code = 7,
    })
    mock_angle("approve", "Delete angle approves.")
    mock_meta("converge: Retry the failed structural angle with a concrete blocker.")

    local result = run_decide(proposal(), opts("codex-fails"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    local invalid = false
    for _, digest in ipairs(result.raises[1].payload.angle_digests) do
      if digest.verdict == "invalid" then
        invalid = true
      end
    end
    t.eq(invalid, true)
    t.eq(#codex_calls(), 4)
  end,

  test_unparseable_output_raises_consensus_converge_with_default_question = function()
    t.mock_command("codex exec", { stdout = "no verdict here", exit_code = 0 })
    t.mock_command("codex exec", { stdout = "still nothing useful", exit_code = 0 })
    t.mock_command("codex exec", { stdout = "garbage output", exit_code = 0 })
    mock_meta("malformed")

    local result = run_decide(proposal(), opts("unparseable"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.is_true(result.raises[1].payload.narrowed_question:find("Resolve the concrete disagreement", 1, true) ~= nil)
    t.eq(#codex_calls(), 4)
  end,

  test_missing_source_ref_fails_closed_without_codex = function()
    local result = run_decide(proposal({ source_ref = false }), opts("no-source-ref"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    -- fail-closed BEFORE spawning any codex angle
    t.eq(#codex_calls(), 0)
  end,

  test_angles_override_runs_only_named_angles = function()
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("approve", "Delete angle approves.")

    local result = run_decide(proposal({ angles = { "minimal", "delete" } }), opts("angles-override"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(#result.raises[1].payload.angle_results, 2)

    local calls = codex_calls()
    t.eq(#calls, 2)
    t.is_true(has_stdin_fragment(calls, "Angle: minimal"))
    t.is_true(has_stdin_fragment(calls, "Angle: delete"))
  end,

  test_same_dedup_key_skips_second_run = function()
    local run_opts = opts("cache-hit")
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("approve", "Delete angle approves.")

    local first = run_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    -- identical dedup_key -> idempotent skip, no new codex calls
    local second = run_decide(proposal(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(#codex_calls(), 3)
  end,

  test_new_version_reruns_consensus = function()
    local run_opts = opts("new-version")
    mock_angle("approve", "Minimal angle approves.")
    mock_angle("approve", "Structural angle approves.")
    mock_angle("approve", "Delete angle approves.")

    local first = run_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42-v1")

    -- a new version (different dedup_key) re-derives consensus instead of being skipped
    mock_angle("approve", "Minimal angle approves again.")
    mock_angle("approve", "Structural angle approves again.")
    mock_angle("approve", "Delete angle approves again.")

    local second = run_decide(proposal({ dedup_key = "proposal-42-v2" }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42-v2")
    t.eq(#codex_calls(), 6)
  end,
}
