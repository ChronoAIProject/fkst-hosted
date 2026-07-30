local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local conv_reconcile = require("devloop.convergence.reconcile")
local conv_rounds = require("devloop.convergence.rounds")
local devloop_state = require("devloop.state")
local S = {}
local comment_strings = require("devloop.strings")

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"

function M.build_reconcile_label_request(repo, issue_number, reconcile)
  return requests_labels.build_state_label_request(repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
      "reconcile",
      "label",
      tostring(reconcile.dedup_key),
    }),
    reconcile.source_ref
  )
end

function M.build_review_reconcile_label_request(repo, issue_number, review_reconcile)
  return requests_labels.build_state_label_request(repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
      "review-reconcile",
      "label",
      tostring(review_reconcile.dedup_key),
    }),
    review_reconcile.source_ref
  )
end

function M.build_fix_reconcile_label_request(repo, issue_number, fix_reconcile)
  return requests_labels.build_state_label_request(repo,
    issue_number,
    "blocked",
    base_ids.dedup_key({
      "fix-reconcile",
      "label",
      tostring(fix_reconcile.dedup_key),
    }),
    fix_reconcile.source_ref
  )
end

function M.build_reconcile_comment_request(repo, issue_number, reconcile, action, reason, state_version)
  local version = state_version or conv_reconcile.reconcile_state_version(reconcile.base_version, reconcile.round)
  local marker = conv_reconcile.reconcile_marker(reconcile.proposal_id, reconcile.base_version, reconcile.round, action, reconcile.terminal_cause)
  local state_marker = devloop_state.state_marker(reconcile.proposal_id, "blocked", version)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  return m_claims.attach_issue_claim({
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = comment_strings.comment_string(M, "reconcile_action_prefix") .. tostring(action)
      .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
      .. "\n\n"
      .. state_marker .. "\n" .. marker
      .. "\n" .. ai_sentinel,
    dedup_key = base_ids.dedup_key({
      "reconcile",
      "comment",
      tostring(reconcile.dedup_key),
    }),
    source_ref = base_ids.normalize_source_ref(reconcile.source_ref),
  }, reconcile.source_ref)
end

--- One comment that both AMENDS the spec and re-enters the loop.
--
-- The first line is the `fkst: reintake` operator command, because the state
-- machine treats terminal states as sticky and `blocked` declares reintake as its
-- documented re-entry edge -- inventing a backwards transition would fight the
-- CAS guards instead of using them. The comment is authored by the trusted bot,
-- so `operator_command_fact` accepts it exactly as it accepts a human's.
--
-- The directive deliberately POINTS AT the convergence round comment rather than
-- copying the narrowed question and angle stances into this body. Consensus
-- re-reads the issue and all its comments from source on the next lap, so the
-- detail is already available; copying it would duplicate content into a payload,
-- which is the pattern CLAUDE.md explicitly rules out.
function M.build_auto_refine_comment_request(repo, issue_number, reconcile, refine_round, cause, state_version, budget)
  local marker = conv_rounds.auto_refine_marker(reconcile.proposal_id, refine_round, cause)
  local body = "fkst: reintake"
    .. "\n\n" .. ai_sentinel
    .. "\n\n**Auto-refinement " .. tostring(refine_round) .. "/"
    .. tostring(budget or conv_rounds.DEFAULT_MAX_AUTO_REFINEMENTS)
    .. "** — consensus reached terminal cause `" .. tostring(cause) .. "` at round "
    .. tostring(reconcile.round) .. ", so this item is being refined and re-run"
    .. " rather than left blocked for a human to notice."
    .. "\n\nBefore the next consensus lap, amend this issue's specification to"
    .. " resolve the narrowed question recorded in the convergence round comment"
    .. " above: tighten every rule it reports as contradictory or under-specified,"
    .. " and add the tests it asks for. State the amendment explicitly in a comment"
    .. " so the next lap reads it as part of the spec."
    .. "\n\nDo NOT widen scope to make the disagreement go away, and do not restate"
    .. " the whole spec — amend only what the review actually challenged."
    .. "\n\n" .. marker
  return m_claims.attach_issue_claim({
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = body,
    dedup_key = base_ids.dedup_key({
      "auto-refine",
      "comment",
      tostring(reconcile.dedup_key),
      tostring(refine_round),
    }),
    source_ref = base_ids.normalize_source_ref(reconcile.source_ref),
  }, reconcile.source_ref)
end

function M.build_fix_reconcile_comment_request(repo, issue_number, fix_reconcile, action, reason)
  local version = conv_reconcile.fix_reconcile_state_version(fix_reconcile.issue_version)
  local marker = conv_reconcile.fix_reconcile_marker(fix_reconcile.proposal_id, fix_reconcile.issue_version, action)
  local state_marker = devloop_state.state_marker(fix_reconcile.proposal_id, "blocked", version)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = devloop_base.parse_pr_source_ref(fix_reconcile.source_ref)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, comment_strings.comment_string(M, "fix_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, base_ids.dedup_key({
    "fix-reconcile",
    "comment",
    tostring(fix_reconcile.dedup_key),
  }), fix_reconcile.source_ref)
end

function M.build_review_reconcile_comment_request(repo, issue_number, review_reconcile, action, reason, state_version)
  local version = state_version or conv_reconcile.review_reconcile_state_version(review_reconcile.issue_version, review_reconcile.round)
  local marker = conv_reconcile.review_reconcile_marker(review_reconcile.proposal_id, review_reconcile.issue_version, review_reconcile.round, action, review_reconcile.terminal_cause)
  local state_marker = devloop_state.state_marker(review_reconcile.proposal_id, "blocked", version)
  local safe_reason = devloop_base.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = devloop_base.parse_pr_source_ref(review_reconcile.source_ref)
  return entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, comment_strings.comment_string(M, "review_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. comment_strings.comment_string(M, "reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, base_ids.dedup_key({
    "review-reconcile",
    "comment",
    tostring(review_reconcile.dedup_key),
  }), review_reconcile.source_ref)
end
end

return S
