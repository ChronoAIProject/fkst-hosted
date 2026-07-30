local parsers_misc = require("devloop.parsers.misc")
local devloop_base = require("devloop.base")
local shared = require("devloop.convergence.shared")
local transition_version = require("contract.transition_version")
local contract_time = require("contract.time")
local C = {}

local valid_round = shared.valid_round
local max_digest_len = shared.max_digest_len
local max_attr_len = shared.max_attr_len
local max_question_len = shared.max_question_len
local safe_attr = shared.safe_attr
local decode_attr = shared.decode_attr
local decode_angle_replay = shared.decode_angle_replay
local encode_angle_replay = shared.encode_angle_replay
local decode_findings_record = shared.decode_findings_record
local encode_findings_record = shared.encode_findings_record
local converge_question_digest = shared.converge_question_digest
local converge_verdicts_digest = shared.converge_verdicts_digest
local converge_angles_digest = shared.converge_angles_digest
local attr = shared.attr
local is_digest = shared.is_digest
local is_bounded_attr = shared.is_bounded_attr
local normalize_findings_record = shared.normalize_findings_record

local function converge_record_map(comments, kind, matches)
  local records_by_round = {}
  if type(comments) ~= "table" then
    return {}
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. kind .. ":v1.-%-%->"
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(comments)) do
    for marker in parsers_misc._comment_body(comment):gmatch(marker_pattern) do
      local round = valid_round(attr(marker, "round"))
      local question = attr(marker, "question")
      local verdicts = attr(marker, "verdicts")
      local dedup = attr(marker, "dedup")
      local narrowed_question = decode_attr(attr(marker, "narrowed_question"))
      local angle_digests = decode_angle_replay(attr(marker, "angle_digests"))
      local findings_record = decode_findings_record(attr(marker, "findings_record"))
      local essence_stall = attr(marker, "essence_stall") == "true"
      local version = attr(marker, "version")
      if round ~= nil
        and matches(marker)
        and is_digest(question)
        and is_digest(verdicts)
        and is_bounded_attr(nil, dedup, devloop_base._max_dedup_len) then
        records_by_round[round] = {
          round = round,
          question = question,
          verdicts = verdicts,
          dedup = dedup,
          version = version,
          comment_created_at = parsers_misc._comment_created_at(comment),
          narrowed_question = narrowed_question,
          angle_digests = angle_digests,
          findings_record = findings_record,
          essence_stall = essence_stall,
        }
      end
    end
  end

  local facts = {}
  for _, record in pairs(records_by_round) do
    table.insert(facts, record)
  end
  table.sort(facts, function(a, b)
    return a.round < b.round
  end)
  return facts
end
function C.append_converge_round_fact(facts, round, narrowed_question, angle_digests, dedup_key, findings_record, essence_stall)
  local copied = {}
  for _, fact in ipairs(facts or {}) do
    table.insert(copied, fact)
  end
  local normalized_findings = normalize_findings_record(findings_record)
  table.insert(copied, {
    round = round,
    question = converge_question_digest(narrowed_question),
    verdicts = converge_verdicts_digest(angle_digests),
    dedup = dedup_key,
    findings_record = normalized_findings,
    essence_stall = essence_stall == true,
  })
  return copied
end

function C.has_essence_stall(facts)
  for _, fact in ipairs(facts or {}) do
    if type(fact) == "table" and fact.essence_stall == true then
      return true
    end
  end
  return false
end

function C.continuation_budget_exhausted(facts)
  return C.max_converge_round(facts) >= 1
end

-- Auto-refinement budget. A convergence terminal used to be the end of the line:
-- the reconcile dropped the item to `blocked` and a human had to notice and issue
-- `fkst: reintake` by hand. That makes a self-driving session stop on the first
-- disagreement, which is the common case for a large generated spec.
--
-- Instead the loop can refine and retry ITSELF, bounded -- but only when the
-- session asks for it.
--
-- The default is 0: OFF. Self-refinement changes what a blocked item does, and
-- that is the session owner's call, not the platform's. With it off, consensus
-- blocks and a human resolves it, exactly as before this existed. A session opts
-- in from its trigger:
--
--   ### Package Env
--   #### github-devloop
--   FKST_DEVLOOP_AUTO_REFINE_MAX=2
--
-- Two is the suggested value for the same reason it used to be the constant: the
-- angles are re-run against an amended spec each time, so a disagreement that
-- survives two amendments is a genuine design question that wants a human, not
-- another lap.
C.DEFAULT_MAX_AUTO_REFINEMENTS = 0

-- Upper bound on what a session may ask for. A budget is only a budget if it is
-- bounded: without this an author could type a number large enough to make the
-- loop effectively unbounded.
C.MAX_AUTO_REFINEMENTS_CEILING = 5

--- The refinement budget this session is configured for.
--
-- Read through `config.read_env`, so the value comes from the session's
-- `### Package Env` (or a manifest default) when set and is 0 otherwise. A value
-- that is not a non-negative integer within the ceiling falls back to the
-- default rather than erroring: a typo must not take a session down, and 0 is
-- the safe reading of "I could not understand what you asked for".
function C.max_auto_refinements(exec)
  local raw = require("devloop.config").read_env("FKST_DEVLOOP_AUTO_REFINE_MAX", exec)
  local parsed = tonumber(raw)
  if parsed == nil or parsed ~= math.floor(parsed) or parsed < 0 then
    return C.DEFAULT_MAX_AUTO_REFINEMENTS
  end
  if parsed > C.MAX_AUTO_REFINEMENTS_CEILING then
    return C.MAX_AUTO_REFINEMENTS_CEILING
  end
  return parsed
end

-- Every terminal cause is refinable, including `external-evidence-required`.
-- That cause means the angles could not settle the question from the issue and
-- the repo alone -- which in an unattended loop is a request for a decision, and
-- a decision is exactly what a refinement pass can record. Stopping instead would
-- wait on a human who is not there. If the missing fact really is unobtainable,
-- the budget below runs out and the loop stops with the full round history intact,
-- so the bound on retrying is the configured budget -- never the cause.
--
-- A cause added to `terminal_causes` is therefore refinable by default; excluding
-- one is a deliberate act that belongs here, with its reason.
function C.is_refinable_cause(value)
  return C.is_terminal_cause(value)
end

local auto_refine_pattern = "fkst:github%-devloop:auto%-refine:v1"

--- Count auto-refinements already recorded on this proposal.
--
-- Counted from durable trusted markers rather than held in memory: a session pod
-- is recycled freely, and a budget that resets on restart is not a budget.
function C.auto_refine_count(comments, proposal_id)
  local seen = 0
  for _, comment in ipairs(comments or {}) do
    local body = type(comment) == "table" and tostring(comment.body or "") or tostring(comment or "")
    if body:find(auto_refine_pattern) ~= nil
      and body:find(tostring(proposal_id or ""), 1, true) ~= nil then
      seen = seen + 1
    end
  end
  return seen
end

--- Whether another refinement is allowed for this proposal.
--
-- `budget` is the resolved value from `C.max_auto_refinements`. Passed in rather
-- than read here so one reconcile pass resolves it once and every message it
-- renders quotes the same number.
function C.auto_refine_budget_remaining(comments, proposal_id, budget)
  local allowed = tonumber(budget) or C.DEFAULT_MAX_AUTO_REFINEMENTS
  return C.auto_refine_count(comments, proposal_id) < allowed
end

function C.auto_refine_marker(proposal_id, refine_round, cause)
  return "<!-- fkst:github-devloop:auto-refine:v1 proposal=\"" .. tostring(proposal_id)
    .. "\" round=\"" .. tostring(refine_round)
    .. "\" cause=\"" .. tostring(cause) .. "\" -->"
end

local terminal_causes = {
  ["external-evidence-required"] = true,
  ["no-semantic-progress"] = true,
  ["evidence-continuation-budget-exhausted"] = true,
}

function C.is_terminal_cause(value)
  return terminal_causes[tostring(value)] == true
end

function C.terminal_cause(facts, current_round)
  if C.has_essence_stall(facts) then
    return "external-evidence-required"
  end
  if C.is_true_stall(facts, current_round) then
    return "no-semantic-progress"
  end
  if C.continuation_budget_exhausted(facts) then
    return "evidence-continuation-budget-exhausted"
  end
  return nil
end

function C.converge_base_version(consensus_dedup)
  return transition_version.strip_trailing_loop(consensus_dedup)
end

function C.converge_proposal_base_dedup(consensus_dedup)
  local base_version = C.converge_base_version(consensus_dedup)
  return base_version:match("^consensus:(.+)$") or base_version
end
function C.converge_round_marker(proposal_id, base_version, source_ref_digest, round, consensus_dedup, narrowed_question, angle_digests, findings_record, essence_stall)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid converge round")
  end
  return '<!-- fkst:github-devloop:converge-round:v1 proposal="' .. safe_attr(proposal_id, devloop_base._max_key_len)
    .. '" version="' .. safe_attr(base_version, devloop_base._max_dedup_len)
    .. '" source_ref="' .. safe_attr(source_ref_digest, max_digest_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr(consensus_dedup, devloop_base._max_dedup_len)
    .. '" question="' .. converge_question_digest(narrowed_question)
    .. '" verdicts="' .. converge_verdicts_digest(angle_digests)
    .. '" angles="' .. converge_angles_digest(angle_digests)
    .. '" narrowed_question="' .. safe_attr(narrowed_question, max_question_len)
    .. '" angle_digests="' .. encode_angle_replay(angle_digests)
    .. '" findings_record="' .. encode_findings_record(findings_record)
    .. '" essence_stall="' .. (essence_stall == true and "true" or "false")
    .. '" -->'
end
function C.review_converge_round_marker(M, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest, round, consensus_dedup, narrowed_question, angle_digests, findings_record, essence_stall)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid review converge round")
  end
  local heartbeat_version = M.liveness_heartbeat_version(issue_version, M.liveness_signal_producer_contract("review-converge-round"))
  return '<!-- fkst:github-devloop:review-converge-round:v1 proposal="' .. safe_attr(review_proposal_id, devloop_base._max_key_len)
    .. '" issue_proposal="' .. safe_attr(issue_proposal_id, devloop_base._max_key_len)
    .. '" version="' .. safe_attr(heartbeat_version, devloop_base._max_dedup_len)
    .. '" head_sha="' .. safe_attr(head_sha, max_attr_len)
    .. '" source_ref="' .. safe_attr(source_ref_digest, max_digest_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr(consensus_dedup, devloop_base._max_dedup_len)
    .. '" question="' .. converge_question_digest(narrowed_question)
    .. '" verdicts="' .. converge_verdicts_digest(angle_digests)
    .. '" angles="' .. converge_angles_digest(angle_digests)
    .. '" narrowed_question="' .. safe_attr(narrowed_question, max_question_len)
    .. '" angle_digests="' .. encode_angle_replay(angle_digests)
    .. '" findings_record="' .. encode_findings_record(findings_record)
    .. '" essence_stall="' .. (essence_stall == true and "true" or "false")
    .. '" -->'
end

function C.converge_round_facts(comments, proposal_id, base_version, source_ref_digest)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
      and attr(marker, "version") == tostring(base_version)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(comments, "converge%-round", matches)
end

function C.converge_round_facts_for_source(comments, proposal_id, source_ref_digest)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(comments, "converge%-round", matches)
end

function C.converge_round_facts_for_proposal(comments, proposal_id)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
  end
  return converge_record_map(comments, "converge%-round", matches)
end

function C.converge_round_facts_since(comments, proposal_id, marker_created_at)
  local facts = C.converge_round_facts_for_proposal(comments, proposal_id)
  local epoch_seconds = contract_time.iso_timestamp_epoch_seconds(marker_created_at)
  if epoch_seconds == nil then
    return facts
  end
  local filtered = {}
  for _, fact in ipairs(facts) do
    local fact_seconds = contract_time.iso_timestamp_epoch_seconds(fact.comment_created_at)
    if fact_seconds ~= nil and fact_seconds >= epoch_seconds then
      table.insert(filtered, fact)
    end
  end
  return filtered
end

function C.review_converge_round_facts(M, comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest)
  local heartbeat_version = M.liveness_heartbeat_version(issue_version, M.liveness_signal_producer_contract("review-converge-round"))
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(review_proposal_id)
      and attr(marker, "issue_proposal") == tostring(issue_proposal_id)
      and attr(marker, "version") == tostring(heartbeat_version)
      and attr(marker, "head_sha") == tostring(head_sha)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(comments, "review%-converge%-round", matches)
end

function C.converge_budget_round(comments, proposal_id)
  return C.max_converge_round(C.converge_round_facts_for_proposal(comments, proposal_id))
end

function C.max_converge_round(facts)
  local max_seen = 0
  if type(facts) ~= "table" then
    return max_seen
  end
  for _, fact in ipairs(facts) do
    local round = valid_round(type(fact) == "table" and fact.round or nil)
    if round ~= nil and round > max_seen then
      max_seen = round
    end
  end
  return max_seen
end

function C.has_converge_round_marker(comments, proposal_id, base_version, source_ref_digest, round)
  local n = valid_round(round)
  if n == nil then
    return false
  end
  for _, fact in ipairs(C.converge_round_facts(comments, proposal_id, base_version, source_ref_digest)) do
    if fact.round == n then
      return true
    end
  end
  return false
end
function C.has_review_converge_round_marker(M, comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest, round)
  local n = valid_round(round)
  if n == nil then
    return false
  end
  for _, fact in ipairs(C.review_converge_round_facts(M, comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest)) do
    if fact.round == n then
      return true
    end
  end
  return false
end

function C.is_true_stall(facts, current_round)
  local round = valid_round(current_round)
  if round == nil or round < 3 or type(facts) ~= "table" then
    return false
  end

  local by_round = {}
  for _, fact in ipairs(facts) do
    if type(fact) == "table" then
      local fact_round = valid_round(fact.round)
      if fact_round ~= nil then
        by_round[fact_round] = fact
      end
    end
  end

  local current = by_round[round]
  local previous = by_round[round - 1]
  local before_previous = by_round[round - 2]
  if current == nil or previous == nil or before_previous == nil then
    return false
  end

  return current.verdicts == previous.verdicts
    and previous.verdicts == before_previous.verdicts
end

return C
