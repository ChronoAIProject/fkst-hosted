local core = require("core")
local t = fkst.test
require("tests.cache_seed_helpers")
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local stance_label = "⟦FKST:STANCE⟧"
local angle_roles = { teleology = true, parsimony = true, fidelity = true }

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/consensus/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
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
    angles = { "teleology", "parsimony", "fidelity" },
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

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = value,
    },
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

local function assert_call_contains(calls, expected)
  for _, call in ipairs(calls) do
    if tostring(call.stdin or ""):find(expected, 1, true) ~= nil then
      return
    end
  end
  error("missing codex stdin fragment: " .. expected)
end

local function count_verdicts(items, verdict)
  local count = 0
  for _, item in ipairs(items or {}) do
    if item.verdict == verdict then
      count = count + 1
    end
  end
  return count
end

local function assert_judgment_worktree(call, role)
  t.is_true(call.rendered:find(" -C ", 1, true) ~= nil)
  t.is_true(call.rendered:find("/judgment-worktrees/consensus-" .. role, 1, true) ~= nil)
  t.is_nil(call.rendered:find("/worktrees/", 1, true))
end

local function judgment_call(role)
  for _, call in ipairs(codex_calls()) do
    if call.rendered:find("/judgment-worktrees/consensus-" .. role, 1, true) ~= nil then
      return call
    end
  end
  return nil
end

local function assert_judgment_dir_created_without_permission_control(count)
  local seen = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("mkdir -p", 1, true) ~= nil
      and call.rendered:find("/judgment-worktrees/consensus-", 1, true) ~= nil then
      seen = seen + 1
      t.is_nil(call.rendered:find("chmod", 1, true))
    end
  end
  t.eq(seen, count)
end

local function mock_judgment_runtime()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/consensus/runtime",
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

local function angle_mock_pattern(angle)
  if angle == nil then
    return "codex exec"
  end
  return "consensus-angle-" .. tostring(angle)
end

local function mock_angle(angle, verdict, reply, exit_code)
  mock_judgment_dir()
  local gap = verdict == "reject" and "\n" .. "⟦FKST:GAP⟧ " .. tostring(reply):sub(1, 80) or ""
  t.mock_command(angle_mock_pattern(angle), {
    stdout = verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply .. gap .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function rebuttal_mock_pattern(angle)
  return "consensus-rebuttal-" .. tostring(angle)
end

local function mock_rebuttal(angle, stance, verdict, reply, peer_claim, exit_code)
  mock_judgment_dir()
  local stance_line = stance_label .. " " .. tostring(stance)
  if stance == "update" and peer_claim ~= nil then
    stance_line = stance_line .. " because " .. tostring(peer_claim)
  end
  local gap = verdict == "reject" and "\n" .. "⟦FKST:GAP⟧ " .. tostring(reply):sub(1, 80) or ""
  t.mock_command(rebuttal_mock_pattern(angle), {
    stdout = stance_line .. "\n" .. verdict_label .. " " .. verdict .. "\n" .. reply_label .. " " .. reply .. gap .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function mock_rebuttal_defend(angle, verdict, reply)
  mock_rebuttal(angle, "defend", verdict, reply)
end

local function mock_synthesis(line, exit_code)
  mock_judgment_dir()
  t.mock_command("consensus-synthesis-proposal", {
    stdout = tostring(line or "") .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function mock_synthesis_repair(line, exit_code)
  mock_judgment_dir()
  t.mock_command("consensus-synthesis-repair-proposal", {
    stdout = tostring(line or "") .. "\n",
    stderr = "",
    exit_code = exit_code or 0,
  })
end

return {
  test_all_angles_approve_raises_consensus_reached = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

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
    t.eq(result.raises[1].payload.angle_results[1].angle, "teleology")
    t.eq(result.raises[1].payload.angle_results[2].angle, "parsimony")
    t.eq(result.raises[1].payload.angle_results[3].angle, "fidelity")

    local calls = codex_calls()
    t.eq(#calls, 3)
    assert_call_contains(calls, "Angle: teleology")
    assert_call_contains(calls, "Angle: parsimony")
    assert_call_contains(calls, "Angle: fidelity")
    assert_call_contains(calls, "source_ref.ref: demo/consensus/42")
    assert_call_contains(calls, "fetch-source --ref demo/consensus/42 --full")
    local teleology_call = judgment_call("angle-teleology")
    local parsimony_call = judgment_call("angle-parsimony")
    local fidelity_call = judgment_call("angle-fidelity")
    t.is_true(teleology_call ~= nil)
    t.is_true(parsimony_call ~= nil)
    t.is_true(fidelity_call ~= nil)
    assert_judgment_worktree(teleology_call, "angle-teleology")
    assert_judgment_worktree(parsimony_call, "angle-parsimony")
    assert_judgment_worktree(fidelity_call, "angle-fidelity")
    assert_judgment_dir_created_without_permission_control(3)
    t.is_true(teleology_call.stdin:find("Angle: teleology", 1, true) ~= nil)
    t.is_true(teleology_call.stdin:find("source_ref.ref: demo/consensus/42", 1, true) ~= nil)
    t.is_true(teleology_call.stdin:find("fetch-source --ref demo/consensus/42 --full", 1, true) ~= nil)
    t.is_true(teleology_call.stdin:find("Do not clone, checkout, fetch with git", 1, true) ~= nil)
    t.is_true(parsimony_call.stdin:find("Angle: parsimony", 1, true) ~= nil)
    t.is_true(fidelity_call.stdin:find("Angle: fidelity", 1, true) ~= nil)
  end,

  test_codex_stdin_carries_fetch_instruction_not_full_body = function()
    local full_tail = "FULL_BODY_TAIL_MUST_NOT_REACH_CODEX"
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

    local result = run_decide(proposal({
      body = "Brief only.",
      content_fetch = "fetch-source --ref demo/consensus/42 --full",
      context = nil,
      full_body = string.rep("x", 16000) .. full_tail,
    }), opts("stdin-fetch-not-full-body"))

    t.eq(result.exit_code, 0)
    local calls = codex_calls()
    t.eq(#calls, 3)
    local teleology_call = judgment_call("angle-teleology")
    t.is_true(teleology_call.stdin:find("Brief only.", 1, true) ~= nil)
    t.is_true(teleology_call.stdin:find("fetch-source --ref demo/consensus/42 --full", 1, true) ~= nil)
    t.is_nil(teleology_call.stdin:find(full_tail, 1, true))
  end,

  test_codex_stdin_resolves_runtime_cache_context_manifest = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    local run_opts = opts("stdin-runtime-cache-context")
    local root = run_opts.env.FKST_RUNTIME_ROOT
    os.execute("mkdir -p " .. shell_single_quote(root .. "/ctx"))
    local issue = assert(io.open(root .. "/ctx/issue.json", "w"))
    issue:write("issue")
    issue:close()
    local diff = assert(io.open(root .. "/ctx/diff.patch", "w"))
    diff:write("diff")
    diff:close()
    local notice = assert(io.open(root .. "/ctx/UNTRUSTED-NOTICE.txt", "w"))
    notice:write("notice")
    notice:close()
    seed_cache("consensus-test/context", "Untrusted notice: " .. root .. "/ctx/UNTRUSTED-NOTICE.txt\nIssue JSON: " .. root .. "/ctx/issue.json\nPR diff patch: " .. root .. "/ctx/diff.patch", run_opts)

    local result = run_decide(proposal({
      content_fetch = "runtime-cache:consensus-test/context",
    }), run_opts)

    t.eq(result.exit_code, 0)
    local calls = codex_calls()
    t.eq(#calls, 3)
    local teleology_call = judgment_call("angle-teleology")
    t.is_true(teleology_call.stdin:find(root .. "/ctx/issue.json", 1, true) ~= nil)
    t.is_true(teleology_call.stdin:find(root .. "/ctx/diff.patch", 1, true) ~= nil)
    t.is_nil(teleology_call.stdin:find("runtime-cache:consensus-test/context", 1, true))
  end,

  test_runtime_cache_context_manifest_missing_file_ack_drops_without_judgment = function()
    mock_judgment_runtime()
    local run_opts = opts("stdin-runtime-cache-missing-file")
    seed_cache("consensus-test/missing-context", "Issue JSON: /tmp/fkst-packages-test/consensus/missing-file.json", run_opts)

    local result = run_decide(proposal({
      content_fetch = "runtime-cache:consensus-test/missing-context",
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_runtime_cache_context_cache_miss_is_terminal_ack_drop = function()
    mock_judgment_runtime()

    local result = run_decide(proposal({
      content_fetch = "runtime-cache:consensus-test/stale-missing-context",
    }), opts("stdin-runtime-cache-stale-miss"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_runtime_cache_context_unreadable_manifest_file_is_terminal_ack_drop = function()
    mock_judgment_runtime()
    local run_opts = opts("stdin-runtime-cache-stale-file")
    local root = run_opts.env.FKST_RUNTIME_ROOT
    os.execute("mkdir -p " .. shell_single_quote(root .. "/ctx"))
    local issue = assert(io.open(root .. "/ctx/issue.json", "w"))
    issue:write("issue")
    issue:close()
    local notice = assert(io.open(root .. "/ctx/UNTRUSTED-NOTICE.txt", "w"))
    notice:write("notice")
    notice:close()
    seed_cache("consensus-test/stale-file", "Untrusted notice: " .. root .. "/ctx/UNTRUSTED-NOTICE.txt\nIssue JSON: " .. root .. "/ctx/issue.json", run_opts)
    os.remove(root .. "/ctx/issue.json")

    local result = run_decide(proposal({
      content_fetch = "runtime-cache:consensus-test/stale-file",
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_unanimous_abstain_raises_consensus_converge = function()
    mock_judgment_runtime()
    mock_angle("teleology", "abstain", "Teleology angle needs narrower scope.")
    mock_angle("parsimony", "abstain", "Parsimony angle needs clearer boundaries.")
    mock_angle("fidelity", "abstain", "Fidelity angle needs proof the scope is necessary.")
    mock_rebuttal_defend("teleology", "abstain", "Teleology still needs narrower scope.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still needs clearer boundaries.")
    mock_rebuttal_defend("fidelity", "abstain", "Fidelity still needs proof the scope is necessary.")
    mock_synthesis("converge: narrowed scope remains unresolved + inspect the requested scope evidence")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/all-abstain" }), opts("all-abstain"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(result.raises[1].payload.narrowed_question, "narrowed scope remains unresolved + inspect the requested scope evidence")
    t.eq(#codex_calls(), 7)
  end,

  test_split_verdicts_spawn_synthesis_and_raise_consensus_converge = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle needs one blocker resolved.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still needs one blocker resolved.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("converge: parsimony concern remains unresolved + inspect the retry-boundary evidence")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/split" }), opts("split"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(result.raises[1].payload.schema, "consensus.consensus_converge.v1")
    t.eq(result.raises[1].payload.proposal_id, "proposal-42")
    t.eq(result.raises[1].payload.dedup_key, "consensus:proposal-42-v1/split")
    t.eq(result.raises[1].payload.round, 0)
    t.eq(result.raises[1].payload.narrowed_question, "parsimony concern remains unresolved + inspect the retry-boundary evidence")
    t.eq(result.raises[1].payload.source_ref.kind, "proposal")
    t.eq(result.raises[1].payload.source_ref.ref, "demo/consensus/42")
    t.eq(#result.raises[1].payload.angle_digests, 3)
    t.eq(count_verdicts(result.raises[1].payload.angle_digests, "approve"), 2)
    t.eq(count_verdicts(result.raises[1].payload.angle_digests, "abstain"), 1)
    t.is_nil(result.raises[1].payload.body)
    t.is_nil(result.raises[1].payload.angle_results)
    t.is_nil(result.raises[1].payload.decision)
    local calls = codex_calls()
    t.eq(#calls, 7)
    local synthesis_call = judgment_call("synthesis")
    assert_judgment_worktree(synthesis_call, "synthesis")
    t.is_true(synthesis_call.stdin:find("Phase B transcripts:", 1, true) ~= nil)
    t.is_true(synthesis_call.stdin:find("Phase R transcripts:", 1, true) ~= nil)
    t.is_true(synthesis_call.stdin:find("You are running in an empty runtime scratch directory", 1, true) ~= nil)
  end,

  test_split_verdicts_rebuttal_unanimity_raises_consensus_reached = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology approves because the purpose forces it.")
    mock_angle("parsimony", "abstain", "Parsimony needs the retry boundary named.")
    mock_angle("fidelity", "approve", "Fidelity approves because source_ref is direct.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal("parsimony", "update", "approve", "Parsimony now approves after teleology named the purpose.", "teleology purpose claim")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/split-rebuttal-unanimity" }), opts("split-rebuttal-unanimity"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(result.raises[1].payload.verdict_path, "post-rebuttal-unanimity")
    t.eq(result.raises[1].payload.verified_moves, 0)
    t.eq(#result.raises[1].payload.p1_verdicts, 3)
    t.eq(result.raises[1].payload.p1_verdicts[2].angle, "parsimony")
    t.eq(result.raises[1].payload.p1_verdicts[2].verdict, "abstain")
    t.eq(#result.raises[1].payload.p2_verdicts, 3)
    t.eq(count_verdicts(result.raises[1].payload.p2_verdicts, "approve"), 3)
    t.eq(#codex_calls(), 6)
    t.eq(judgment_call("synthesis"), nil)
    t.eq(judgment_call("synthesis-repair"), nil)
    local parsimony = judgment_call("rebuttal-parsimony")
    assert_judgment_worktree(parsimony, "rebuttal-parsimony")
    t.is_true(parsimony.stdin:find("Your locked Phase B output:", 1, true) ~= nil)
    t.is_true(parsimony.stdin:find("Peer Phase B outputs:", 1, true) ~= nil)
    t.is_true(parsimony.stdin:find("teleology purpose claim", 1, true) == nil)
    assert_judgment_dir_created_without_permission_control(6)
  end,

  test_duplicate_converge_delivery_redecides_but_emits_stable_dedup_key = function()
    local run_opts = opts("duplicate-converge-delivery")
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle needs one blocker resolved.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still needs one blocker resolved.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("converge: parsimony concern remains unresolved + inspect the retry-boundary evidence")

    local first = run_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].queue, "consensus_converge")
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42-v1")
    t.eq(first.raises[1].payload.narrowed_question, "parsimony concern remains unresolved + inspect the retry-boundary evidence")

    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves on replay.")
    mock_angle("parsimony", "abstain", "Parsimony angle still needs one blocker resolved.")
    mock_angle("fidelity", "approve", "Fidelity angle approves on replay.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves on replay.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still needs one blocker resolved.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves on replay.")
    mock_synthesis("converge: replay disagreement remains unresolved + inspect the downstream dedup key")

    local second = run_decide(proposal(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].queue, "consensus_converge")
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42-v1")
    t.eq(second.raises[1].payload.round, 0)
    t.eq(second.raises[1].payload.source_ref.kind, "proposal")
    t.eq(second.raises[1].payload.source_ref.ref, "demo/consensus/42")
    t.eq(second.raises[1].payload.narrowed_question, "replay disagreement remains unresolved + inspect the downstream dedup key")
    t.eq(#codex_calls(), 14)
  end,

  test_synthesis_parse_failure_retries_once_and_uses_repair_result = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle accepts a small adapter.")
    mock_angle("parsimony", "abstain", "Parsimony angle wants the retry boundary explicit.")
    mock_angle("fidelity", "approve", "Fidelity angle accepts removing duplicate wiring.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still accepts a small adapter.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still wants the retry boundary explicit.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still accepts removing duplicate wiring.")
    mock_synthesis("⟦FKST:PLAN⟧ Keep the adapter, make retry ownership explicit, and remove duplicate wiring.")
    mock_synthesis_repair("converge: retry ownership remains unresolved + inspect the retry owner record")

    local result = run_decide(proposal(), opts("split-synthesis-repair"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(result.raises[1].payload.narrowed_question, "retry ownership remains unresolved + inspect the retry owner record")
    t.eq(#codex_calls(), 8)
    local repair = judgment_call("synthesis-repair")
    assert_judgment_worktree(repair, "synthesis-repair")
    t.is_true(repair.stdin:find("Repair attempt:", 1, true) ~= nil)
    t.eq(judgment_call("angle-teleology") ~= nil, true)
    t.eq(judgment_call("rebuttal-teleology") ~= nil, true)
  end,

  test_synthesis_second_parse_failure_converges_without_default_question = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle needs framing.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still needs framing.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("⟦FKST:PLAN⟧")
    mock_synthesis_repair("malformed")

    local result = run_decide(proposal(), opts("malformed-synthesis"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.is_true(result.raises[1].payload.narrowed_question:find("synthesis-parse-failed", 1, true) ~= nil)
    t.is_nil(result.raises[1].payload.narrowed_question:find("Resolve the concrete disagreement", 1, true))
    t.eq(#codex_calls(), 8)
  end,

  test_gate_mode_any_reject_raises_consensus_reached_reject_with_gap = function()
    mock_judgment_runtime()
    mock_angle("teleology", "reject", "Teleology angle rejects the diff.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "comment", "Fidelity angle has advisory feedback.")

    local result = run_decide(proposal({ verdict_mode = "gate" }), opts("gate-any-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "reject")
    t.eq(result.raises[1].payload.blocking_gap, "Teleology angle rejects the diff.")
    t.eq(#codex_calls(), 3)
  end,

  test_gate_mode_approve_with_comment_raises_consensus_reached_approve = function()
    mock_judgment_runtime()
    mock_angle("teleology", "comment", "Teleology angle notes naming could improve.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "abstain", "Fidelity angle cannot judge.")

    local result = run_decide(proposal({ verdict_mode = "gate" }), opts("gate-approve-comment"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.decision, "approve")
    t.is_true(result.raises[1].payload.body:find("Advisory (non-blocking):", 1, true) ~= nil)
    t.eq(#codex_calls(), 3)
  end,

  test_synthesis_reached_after_split_raises_consensus_reached = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle abstains but accepts the narrowed framing.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal("parsimony", "update", "abstain", "Parsimony still abstains after teleology synthesis claim.", "teleology synthesis claim")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("reached:approve approve the narrowed framing\nverified-move: angle=parsimony phase=P2 citation=teleology synthesis claim")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/split-synthesis-reached" }), opts("split-synthesis-reached"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_reached")
    t.eq(result.raises[1].payload.schema, "consensus.consensus_reached.v1")
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(result.raises[1].payload.framing, "approve the narrowed framing")
    t.eq(result.raises[1].payload.verdict_path, "synthesis")
    t.eq(result.raises[1].payload.verified_moves, 1)
    t.eq(result.raises[1].payload.p1_verdicts[2].verdict, "abstain")
    t.eq(result.raises[1].payload.p2_verdicts[2].verdict, "abstain")
    t.eq(result.raises[1].payload.body:find("Meta-judge framing:", 1, true), nil)
    t.eq(#codex_calls(), 7)
  end,

  test_synthesis_reached_with_failed_angle_falls_back_to_consensus_converge = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", {
      stderr = "forced failure",
      exit_code = 7,
    })
    mock_angle("fidelity", "abstain", "Fidelity angle abstains.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony cannot judge after the failed P1.")
    mock_rebuttal_defend("fidelity", "abstain", "Fidelity still abstains.")
    mock_synthesis("reached:approve approve the narrowed framing")

    local run_opts = opts("split-synthesis-reached-degraded")
    local result = run_decide(proposal({
      dedup_key = "proposal-42-v1/split-synthesis-reached-degraded",
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.is_nil(cache_get(core.reached_cache_key("proposal-42-v1/split-synthesis-reached-degraded")))
    t.eq(#codex_calls(), 7)
  end,

  test_synthesis_reached_with_failed_angle_falls_back_to_consensus_converge_in_gate_mode = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", {
      stderr = "forced failure",
      exit_code = 7,
    })
    mock_angle("fidelity", "comment", "Fidelity angle notes a non-blocking concern.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony cannot judge after the failed P1.")
    mock_rebuttal_defend("fidelity", "comment", "Fidelity still has a non-blocking concern.")
    mock_synthesis("reached:approve approve the narrowed framing")

    local run_opts = opts("gate-split-synthesis-reached-degraded")
    local result = run_decide(proposal({
      verdict_mode = "gate",
      dedup_key = "proposal-42-v1/gate-split-synthesis-reached-degraded",
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.is_nil(cache_get(core.reached_cache_key("proposal-42-v1/gate-split-synthesis-reached-degraded")))
    t.eq(#codex_calls(), 7)
  end,

  test_abstain_raises_consensus_converge = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "abstain", "Parsimony angle abstains.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony still abstains.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("converge: parsimony blocker remains unresolved + inspect the named blocker")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/abstain" }), opts("abstain"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.eq(#codex_calls(), 7)
  end,

  test_failed_codex_call_raises_consensus_converge = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", {
      stderr = "forced failure",
      exit_code = 7,
    })
    mock_angle("fidelity", "approve", "Fidelity angle approves.")
    mock_rebuttal_defend("teleology", "approve", "Teleology still approves.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony cannot judge after the failed P1.")
    mock_rebuttal_defend("fidelity", "approve", "Fidelity still approves.")
    mock_synthesis("converge: parsimony angle failed + inspect the failed parsimony evidence")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/codex-fails" }), opts("codex-fails"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    local has_parsimony = false
    for _, digest in ipairs(result.raises[1].payload.angle_digests) do
      if digest.angle == "parsimony" and digest.verdict == "abstain" then
        has_parsimony = true
      end
    end
    t.eq(has_parsimony, true)
    t.eq(#codex_calls(), 7)
  end,

  test_unparseable_output_raises_consensus_converge_with_synthesis_parse_failure = function()
    mock_judgment_runtime()
    mock_judgment_dir()
    t.mock_command("consensus-angle-teleology", { stdout = "no verdict here", exit_code = 0 })
    mock_judgment_dir()
    t.mock_command("consensus-angle-parsimony", { stdout = "still nothing useful", exit_code = 0 })
    mock_judgment_dir()
    t.mock_command("consensus-angle-fidelity", { stdout = "garbage output", exit_code = 0 })
    mock_rebuttal_defend("teleology", "abstain", "Teleology cannot judge malformed P1.")
    mock_rebuttal_defend("parsimony", "abstain", "Parsimony cannot judge malformed P1.")
    mock_rebuttal_defend("fidelity", "abstain", "Fidelity cannot judge malformed P1.")
    mock_synthesis("malformed")
    mock_synthesis_repair("still malformed")

    local result = run_decide(proposal({ dedup_key = "proposal-42-v1/unparseable" }), opts("unparseable"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus_converge")
    t.is_true(result.raises[1].payload.narrowed_question:find("synthesis-parse-failed", 1, true) ~= nil)
    t.is_nil(result.raises[1].payload.narrowed_question:find("Resolve the concrete disagreement", 1, true))
    t.eq(#codex_calls(), 8)
  end,

  test_missing_source_ref_fails_closed_without_codex = function()
    local result = run_decide(proposal({ source_ref = false }), opts("no-source-ref"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    -- fail-closed BEFORE spawning any codex angle
    t.eq(#codex_calls(), 0)
  end,

  test_angles_override_runs_only_named_angles = function()
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

    local result = run_decide(proposal({ angles = { "teleology", "fidelity" } }), opts("angles-override"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.decision, "approve")
    t.eq(#result.raises[1].payload.angle_results, 2)

    local calls = codex_calls()
    t.eq(#calls, 2)
    assert_call_contains(calls, "Angle: teleology")
    assert_call_contains(calls, "Angle: fidelity")
    t.is_true(judgment_call("angle-teleology").stdin:find("Angle: teleology", 1, true) ~= nil)
    t.is_true(judgment_call("angle-fidelity").stdin:find("Angle: fidelity", 1, true) ~= nil)
  end,

  test_same_dedup_key_skips_second_run = function()
    local run_opts = opts("cache-hit")
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

    local first = run_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    -- identical dedup_key -> idempotent skip, no new codex calls
    local second = run_decide(proposal(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(#codex_calls(), 3)
  end,

  test_same_decision_dedup_key_skips_updated_effect_version_refire = function()
    local run_opts = opts("effect-version-refire")
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

    local first = run_decide(proposal({
      dedup_key = "proposal-42/intake/1234567890",
      effect_version = "intake/proposal-42/2026-06-03T01-02-03Z",
    }), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42/intake/1234567890")
    t.eq(first.raises[1].payload.effect_version, "intake/proposal-42/2026-06-03T01-02-03Z")

    local second = run_decide(proposal({
      dedup_key = "proposal-42/intake/1234567890",
      effect_version = "intake/proposal-42/2026-06-03T01-22-03Z",
    }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(#codex_calls(), 3)
  end,

  test_new_version_reruns_consensus = function()
    local run_opts = opts("new-version")
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves.")
    mock_angle("parsimony", "approve", "Parsimony angle approves.")
    mock_angle("fidelity", "approve", "Fidelity angle approves.")

    local first = run_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42-v1")

    -- a new version (different dedup_key) re-derives consensus instead of being skipped
    mock_judgment_runtime()
    mock_angle("teleology", "approve", "Teleology angle approves again.")
    mock_angle("parsimony", "approve", "Parsimony angle approves again.")
    mock_angle("fidelity", "approve", "Fidelity angle approves again.")

    local second = run_decide(proposal({ dedup_key = "proposal-42-v2" }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42-v2")
    t.eq(#codex_calls(), 6)
  end,
}
