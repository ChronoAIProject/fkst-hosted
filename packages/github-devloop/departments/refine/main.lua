-- Author a specification amendment after a convergence block.
--
-- Reconcile no longer writes refinement text; it raises `devloop_refine` and this
-- department does the work. It has no lifecycle row of its own, matching
-- `intake_judge` and `decompose`, which also dispatch codex without one --
-- `blocked` is a budget-bounded recovery hold and is structurally forbidden from
-- dispatching a worker, so a row here is neither possible nor needed.
--
-- The codex call happens OUTSIDE the transition lock. It takes minutes; holding a
-- lock across it would block every other pass on this proposal for the duration.
-- The issue is re-read and re-gated after the call, so a state that moved while
-- the model was thinking cannot be written over.
local saga = require("workflow.saga")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local entity_lib = require("devloop.entity")
local devloop_logging = require("devloop.logging")
local devloop_commands = require("devloop.commands")
local workflow_codex = require("workflow.codex")
local conv_rounds = require("devloop.convergence.rounds")
local conv_refine = require("devloop.convergence.refine")

local spec = {
  consumes = { "devloop_refine" },
  published_seam = { "devloop_refine" },
  produces = { "github-proxy.github_issue_comment_request" },
  stall_window = "2m",
  retry = { max_attempts = 2, base = "5s", cap = "10s" },
}

local AI_SENTINEL = "⟦AI:FKST⟧"

--- The issue's comments, and nothing else.
--
-- Only comments are needed here, and reading them through a comments-only helper
-- avoids threading the composed core into a department -- the pattern the
-- core-param ratchet exists to shrink.
local function read_comments(stdout)
  local decoded = json.decode(stdout or "{}")
  local raw = decoded.comments or {}
  local comments = {}
  for _, comment in ipairs(raw) do
    comments[#comments + 1] = {
      body = tostring(comment.body or ""),
      author_login = tostring((comment.author or {}).login or ""),
      created_at = comment.createdAt or comment.created_at,
    }
  end
  return comments
end


--- The newest convergence round recorded for this proposal.
--
-- Newest across ALL laps, not just the current version: a reintake mints a new
-- version, and the objection we must answer was recorded under the previous one.
local function latest_converge_fact(comments, proposal_id)
  local facts = conv_rounds.converge_round_facts_for_proposal(comments, proposal_id)
  local newest = nil
  for _, fact in ipairs(facts or {}) do
    if newest == nil or tonumber(fact.round or 0) >= tonumber(newest.round or 0) then
      newest = fact
    end
  end
  return newest
end

--- Is this refinement round still unspent?
--
-- The durable marker count is the whole guard. A pod recycle must not grant
-- another lap and a duplicate delivery must not spend one twice, and the count
-- survives both because it is read from the issue, not from memory.
--
-- Deliberately NOT also checking that the state is still `blocked`. The comment
-- this produces leads with `fkst: reintake`, and that command is only a re-entry
-- edge FROM `blocked` -- from any other state it is inert. So a refinement that
-- lands after the state moved adds prose to the thread and changes nothing, which
-- is a better outcome than a cursor read that would have to be kept correct as
-- the lifecycle grows.
local function round_unspent(comments, proposal_id, refine)
  if conv_rounds.auto_refine_count(comments, proposal_id) >= tonumber(refine.refine_round) then
    return false, "this refinement round is already recorded"
  end
  return true, nil
end

local function pipeline(event)
  local refine = event.payload or {}
  if not conv_refine.is_supported_refine(refine) then
    devloop_logging.log_entry("refine", event, "unknown",
      devloop_logging.payload_field(refine, "dedup_key"))
    return
  end

  devloop_logging.log_entry("refine", event, refine.proposal_id, refine.dedup_key)
  local repo, issue_number = base_ids.parse_proposal_id(refine.proposal_id)
  local lock_key = entity_lib.transition_lock_key(refine.proposal_id)
  if lock_key == nil then
    return
  end

  devloop_base.assert_trusted_bot_configured()

  -- Read under the lock, then let it go before the model call.
  local prompt, gate_reason
  with_lock(lock_key, function()
    local view = devloop_commands.gh_issue_view_loop(repo, issue_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: issue-read-failed: refine issue view failed: " .. tostring(view.stderr))
    end
    local comments = read_comments(view.stdout)

    local ok, reason = round_unspent(comments, refine.proposal_id, refine)
    if not ok then
      gate_reason = reason
      return
    end

    local fact = latest_converge_fact(comments, refine.proposal_id)
    if fact == nil then
      gate_reason = "no convergence round to refine against"
      return
    end
    prompt = conv_refine.build_prompt({
      repo = repo,
      issue_number = issue_number,
      terminal_cause = refine.terminal_cause,
      narrowed_question = fact.narrowed_question,
      findings_record = fact.findings_record,
    })
  end)

  if gate_reason ~= nil then
    devloop_logging.log_cas_decision("refine", refine.proposal_id,
      { state = nil, version = nil }, "blocked", "blocked", "skip", gate_reason)
    return
  end

  -- OUTSIDE the lock: minutes-long, and nothing else may wait on it.
  local amendment, failure
  local ok, result = pcall(spawn_codex_sync, workflow_codex.judgment_codex_opts(
    prompt,
    devloop_base.judgment_worktree_with_exec(exec_sync, "refine", refine.dedup_key)
  ))
  if not ok then
    failure = "the refinement run could not be started"
  elseif type(result) == "table" and tonumber(result.exit_code or 0) ~= 0 then
    failure = "the refinement run exited non-zero"
  else
    local stdout = type(result) == "table" and result.stdout or result
    amendment = conv_refine.parse_amendment(stdout)
    if amendment == nil then
      failure = "the refinement run returned no usable amendment"
    end
  end

  with_lock(lock_key, function()
    local view = devloop_commands.gh_issue_view_loop(repo, issue_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: issue-read-failed: refine re-read failed: " .. tostring(view.stderr))
    end
    local comments = read_comments(view.stdout)

    -- Re-gate: another pass may have recorded this round while the model worked.
    local still_ok, reason = round_unspent(comments, refine.proposal_id, refine)
    if not still_ok then
      devloop_logging.log_cas_decision("refine", refine.proposal_id,
        { state = nil, version = nil }, "blocked", "blocked", "skip", reason)
      return
    end

    local body
    if failure ~= nil then
      -- No `fkst: reintake` here on purpose. Re-entering without an amendment is
      -- precisely the bug this department exists to fix, and it would spend a
      -- budget lap for nothing.
      body = conv_refine.build_failure_body({
        ai_sentinel = AI_SENTINEL,
        refine_round = refine.refine_round,
        budget = refine.budget,
        terminal_cause = refine.terminal_cause,
        reason = failure,
      })
    else
      body = conv_refine.build_comment_body({
        ai_sentinel = AI_SENTINEL,
        proposal_id = refine.proposal_id,
        refine_round = refine.refine_round,
        budget = refine.budget,
        terminal_cause = refine.terminal_cause,
        round = refine.round,
        amendment = amendment,
      })
    end

    local request = m_claims.attach_issue_claim({
      schema = "github-proxy.v1",
      repo = repo,
      issue_number = issue_number,
      body = body,
      dedup_key = base_ids.dedup_key({
        "refine",
        "comment",
        tostring(refine.dedup_key),
        failure ~= nil and "failed" or "authored",
      }),
      source_ref = base_ids.normalize_source_ref(refine.source_ref),
    }, refine.source_ref)

    devloop_logging.log_raise("refine", refine.proposal_id,
      "github-proxy.github_issue_comment_request", request)
  end)
end

return saga.department(spec, {
  -- No terminal fact of its own: the amendment comment IS the outcome, and the
  -- durable auto-refine marker is what makes a second delivery a no-op.
  done = function() return false end,
  act = pipeline,
  wrap = devloop_logging.wrap_pipeline_failure,
  name = "refine",
})
