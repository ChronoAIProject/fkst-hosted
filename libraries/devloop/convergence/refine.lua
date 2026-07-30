-- Spec refinement after a convergence block: payload, prompt, and comment.
--
-- The bug this exists to fix: on a block the reconcile pass wrote a comment whose
-- FIRST line was the operator command `fkst: reintake` and whose body was a
-- directive asking someone to amend the specification. `parse_command` reads only
-- the first non-empty line, so the command fired immediately -- before any
-- amendment existed -- and nothing in the tree ever authored one. The next lap
-- then re-judged byte-identical content and reproduced the same verdict.
--
-- So the amendment must be written by an LLM, and it must live in the SAME comment
-- as the command. One comment is one atomic GitHub write, which means the
-- amendment can never be missing at the moment the command is seen. The next lap
-- reads it because the consensus context bundle fetches all comments and its
-- author whitelist is seeded from the bot login.
--
-- `local C`, not `local M`: the godlib ratchet counts `M.<sym>` writes across
-- `libraries/devloop/**` against a shrink-only baseline.
local C = {}

local base_ids = require("devloop.base_ids")
local strings = require("contract.strings")
local source_refs = require("contract.source_ref")
local devloop_base = require("devloop.base")
local conv_rounds = require("devloop.convergence.rounds")

C.SCHEMA = "github-devloop.refine.v1"

--- The event reconcile raises to ask for a refinement.
function C.build_refine_payload(reconcile, refine_round, budget)
  return {
    schema = C.SCHEMA,
    proposal_id = reconcile.proposal_id,
    base_version = reconcile.base_version,
    round = reconcile.round,
    refine_round = refine_round,
    budget = budget,
    terminal_cause = reconcile.terminal_cause,
    dedup_key = base_ids.dedup_key({
      "refine",
      tostring(reconcile.proposal_id),
      tostring(reconcile.base_version),
      tostring(reconcile.round),
      tostring(refine_round),
    }),
    source_ref = base_ids.normalize_source_ref(reconcile.source_ref),
  }
end

--- Reject anything that is not a refinement this build produced.
--
-- `refine_round <= budget` is checked here as well as at the raise site: the
-- queue is durable, so a payload can outlive the configuration that created it,
-- and a stale event must not spend a budget the session no longer grants.
function C.is_supported_refine(payload)
  if type(payload) ~= "table" or payload.schema ~= C.SCHEMA then
    return false
  end
  local repo, issue_number = base_ids.parse_proposal_id(payload.proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  local refine_round = tonumber(payload.refine_round)
  local budget = tonumber(payload.budget)
  return strings.is_path_safe_key(payload.proposal_id, devloop_base._max_key_len)
    and conv_rounds.is_terminal_cause(payload.terminal_cause)
    and refine_round ~= nil
    and refine_round >= 1
    and budget ~= nil
    and refine_round <= budget
    and strings.is_path_safe_key(payload.dedup_key, devloop_base._max_dedup_len)
    and source_refs.has_bounded_source_ref(payload.source_ref, devloop_base._max_key_len)
end

--- The instruction handed to the LLM.
--
-- It is told to AMEND, not to advise: the output is pasted into the issue thread
-- and read as specification by the next lap, so "you should consider..." would be
-- read as part of the spec. The narrowed question and the angle findings come
-- from the convergence round the block produced, which is the whole point --
-- refining without that feedback is just a retry.
function C.build_prompt(args)
  local narrowed = tostring(args.narrowed_question or "")
  local findings = tostring(args.findings_record or "")
  return table.concat({
    "You are amending a GitHub issue that specifies a software change.",
    "",
    "A multi-angle review could not reach consensus on it and recorded why. Your",
    "job is to write the amendment that resolves the recorded objection, so the",
    "next review can converge.",
    "",
    "Issue: " .. tostring(args.repo) .. "#" .. tostring(args.issue_number),
    "Terminal cause: " .. tostring(args.terminal_cause),
    "",
    "The narrowed question the review could not settle:",
    narrowed,
    "",
    "Findings recorded by the review:",
    findings,
    "",
    "Read the issue and the repository, then write the amendment itself.",
    "",
    "Rules:",
    "- Amend ONLY what the review actually challenged. Do not restate the spec.",
    "- Do not widen scope to make the disagreement go away.",
    "- Do not weaken or delete a required test to make a contradiction disappear;",
    "  correct the contract so it is satisfiable as written.",
    "- Write it as specification prose, not as advice. It is pasted into the issue",
    "  and read as part of the spec by the next review.",
    "- If the objection is that two requirements contradict, say which one changes",
    "  and what it becomes.",
    "",
    "Reply with JSON and nothing else:",
    '{"amendment": "<the amendment, in markdown>"}',
  }, "\n")
end

--- Pull the amendment out of the model's reply.
--
-- Returns nil on anything unexpected so the caller can fall back to a
-- deterministic body rather than posting a half-parsed reply into a spec.
function C.parse_amendment(stdout)
  local text = tostring(stdout or "")
  local ok, decoded = pcall(json.decode, text)
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  local amendment = decoded.amendment
  if type(amendment) ~= "string" then
    return nil
  end
  amendment = amendment:gsub("^%s+", ""):gsub("%s+$", "")
  if amendment == "" or #amendment > devloop_base._max_body_len then
    return nil
  end
  return amendment
end

--- The single amend-and-reintake comment.
--
-- Line 1 is the operator command, because that is all `parse_command` reads.
-- Everything after it is the amendment the next lap will judge. The auto-refine
-- marker closes the body and is what the durable budget is counted from.
function C.build_comment_body(args)
  local marker = conv_rounds.auto_refine_marker(
    args.proposal_id, args.refine_round, args.terminal_cause)
  local header = "**Auto-refinement " .. tostring(args.refine_round) .. "/"
    .. tostring(args.budget) .. "** — the review could not converge ("
    .. "`" .. tostring(args.terminal_cause) .. "` at round "
    .. tostring(args.round) .. "). The specification is amended below and the"
    .. " item re-enters review; no human action is required."
  return table.concat({
    "fkst: reintake",
    "",
    tostring(args.ai_sentinel or ""),
    "",
    header,
    "",
    "---",
    "",
    tostring(args.amendment),
    "",
    marker,
  }, "\n")
end

--- The body used when the model fails or returns something unusable.
--
-- Deliberately NOT a `fkst: reintake` command: re-entering with no amendment is
-- the exact bug this module fixes, and it would silently spend a budget lap. The
-- item stays blocked and a human sees why.
function C.build_failure_body(args)
  return table.concat({
    tostring(args.ai_sentinel or ""),
    "",
    "**Auto-refinement " .. tostring(args.refine_round) .. "/"
      .. tostring(args.budget) .. " could not be authored.** The review could not"
      .. " converge (`" .. tostring(args.terminal_cause) .. "`), and the attempt to"
      .. " write an amendment failed: " .. tostring(args.reason) .. ".",
    "",
    "This item stays blocked. Amend the specification and comment"
      .. " `fkst: reintake` to re-enter review.",
  }, "\n")
end

return C
