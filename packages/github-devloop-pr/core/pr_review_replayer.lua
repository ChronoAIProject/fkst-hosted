local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local parsers_misc = require("devloop.parsers.misc")
local payloads_builders = require("devloop.payloads.builders")
local conv_rounds = require("devloop.convergence.rounds")
local conv_reconcile = require("devloop.convergence.reconcile")
local m_facts = require("devloop.markers.facts")
local S = {}
local convergence_shared = require("devloop.convergence.shared")
local check_runs = require("forge.github.check_runs")
local forge_validators = require("devloop.forge_validators")
local transition_version = require("contract.transition_version")
local config = require("devloop.config")
local comment_strings = require("devloop.strings")
local m_builders = require("devloop.markers.builders")

function S.install(M)
local function linked_pr_state(pr)
  return tostring(pr and pr.state or ""):upper()
end

local function merged_head_sha(pr)
  local head_sha = tostring(pr and pr.head_sha or "")
  return forge_validators.is_git_sha(head_sha) and head_sha or nil
end

local function pr_open_state(pr)
  return tostring(pr and pr.state or ""):lower() == "open"
end

local function same_linked_head(link, pr)
  return tostring(pr and pr.head_ref_name or "") == tostring(link and link.branch or "")
    and tostring(pr and pr.base_ref_name or "") == tostring(link and link.base_branch or "")
    and forge_validators.is_git_sha(pr and pr.head_sha)
end

local terminal_linked_pr_action
local mark_child_closed_unmerged
local mark_issue_merged_from_linked_pr
local raise_reviewing_for_current_head

local function linked_open_pr(dept, issue, state, facts, tools, from_state, to_state)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil then
    return nil, nil, tools.log_skip(dept, proposal_id, state, from_state, to_state, "skip-foreign(pr-link)", from_state .. " recovery requires a pr-link marker")
  end
  local current_pr = tools.find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    local terminal = terminal_linked_pr_action(dept, issue, state, proposal_id, link, nil, facts, tools)
    if terminal ~= nil then return nil, nil, terminal end
    return nil, nil, tools.log_skip(dept, proposal_id, state, from_state, to_state, "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local terminal = terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts, tools)
  if terminal ~= nil then return nil, nil, terminal end
  if not pr_open_state(current_pr) then
    return nil, nil, tools.log_skip(dept, proposal_id, state, from_state, to_state, "skip-stale(pr-closed)", "linked PR is not open")
  end
  if not same_linked_head(link, current_pr) then
    return nil, nil, tools.log_skip(dept, proposal_id, state, from_state, to_state, "skip-foreign(pr-head)", "linked PR head/base does not match pr-link marker")
  end
  return link, current_pr, nil
end

local function review_truth_table_unapproved(fact)
  if tonumber(fact and fact.round) == nil or tonumber(fact.round) < 1 then
    return false
  end
  if type(fact.angle_digests) ~= "table" or #fact.angle_digests == 0 then
    return false
  end
  local has_comment = false
  for _, item in ipairs(fact.angle_digests) do
    local verdict = type(item) == "table" and item.verdict or nil
    if verdict == "approve" or verdict == "reject" or verdict == "invalid" then
      return false
    end
    if verdict == "comment" then
      has_comment = true
    elseif verdict == "abstain" then
    else
      return false
    end
  end
  if not has_comment then
    return true
  end
  return tostring(fact.dedup or ""):find("/loop/", 1, true) ~= nil
end

local function append_issue_label_effect(issue, proposal_id, to_state, version, source_ref, effects, key)
  if issue.number == nil then
    return
  end
  table.insert(effects, {
    queue = "github-proxy.github_issue_label_request",
    payload = requests_labels.build_state_label_request(M, issue.repo, issue.number, to_state, key, source_ref),
  })
end

local function add_issue_label_effect(issue, proposal_id, to_state, version, source_ref, effects, dedup_parts)
  if issue.number == nil then
    return
  end
  table.insert(effects, {
    queue = "github-proxy.github_issue_label_request",
    payload = requests_labels.build_state_label_request(M, issue.repo, issue.number, to_state, base_ids.dedup_key(dedup_parts), source_ref),
  })
end

local function fix_comment_from_feedback(issue, pr_number, version, feedback, source_ref)
  return requests_review.build_merge_gate_fix_comment_request(M,
    issue.repo,
    issue.number,
    {
      proposal_id = feedback.proposal_id or feedback.issue_proposal_id or feedback.parent_proposal_id,
      pr_number = pr_number,
      version = version,
      review_proposal_id = feedback.review_proposal_id,
      review_dedup_key = feedback.review_dedup_key,
      reviewed_head_sha = feedback.reviewed_head_sha,
    },
    version,
    feedback.blocking_gap or feedback.review_reason or feedback.reason or "review-result-reject",
    feedback.gate_baseline_sha,
    source_ref,
    feedback.predecessor_set,
    {
      blocking_gap = feedback.blocking_gap,
      gate_failure_excerpt = feedback.gate_failure_excerpt or feedback.review_reason or feedback.reason,
      preserve_nil_gate_failure_excerpt = feedback.gate_failure_excerpt == nil and feedback.review_reason == nil and feedback.reason == nil,
      current_head_sha = feedback.current_head_sha,
    }
  )
end

local function fixing_replay_comment_request(issue, pr_number, fix_payload, feedback, source_ref)
  local request = fix_comment_from_feedback(issue, pr_number, fix_payload.version, {
    proposal_id = fix_payload.proposal_id,
    review_proposal_id = fix_payload.review_proposal_id,
    review_dedup_key = fix_payload.review_dedup_key,
    reviewed_head_sha = fix_payload.reviewed_head_sha,
    blocking_gap = fix_payload.blocking_gap,
    gate_baseline_sha = fix_payload.gate_baseline_sha,
    predecessor_set = fix_payload.predecessor_set,
    gate_failure_excerpt = fix_payload.gate_failure_excerpt,
    review_reason = feedback and feedback.review_reason,
    reason = feedback and feedback.reason,
  }, source_ref)
  request.handoff.dedup_key = fix_payload.dedup_key
  return request
end

local function comments_for_pr_facts(facts, current_pr)
  local comments = {}
  local seen = false
  for _, comment in ipairs(facts and facts.snapshot and facts.snapshot.comments or {}) do
    table.insert(comments, comment)
    seen = true
  end
  for _, comment in ipairs(current_pr and current_pr.comments or {}) do
    table.insert(comments, comment)
    seen = true
  end
  if seen then
    return comments
  end
  return {}
end

local function issue_source_ref(issue)
  if issue.number == nil then
    return issue.source_ref
  end
  return entity_lib.issue_source_ref(issue.repo, issue.number)
end

local function replay_review_result(dept, issue, state, facts, tools, link, current_pr)
  local proposal_id = facts.proposal_id
  local fact = facts["review-result"] or m_facts.review_result_fact(M, comments_for_pr_facts(facts, current_pr), proposal_id, state.version)
  if fact == nil then
    return nil
  end
  local _, review_pr_number, _, reviewed_head_sha = devloop_base.parse_pr_review_proposal_id(fact.review_proposal_id)
  if tostring(review_pr_number or "") ~= tostring(link.pr_number or "")
    or tostring(reviewed_head_sha or "") ~= tostring(current_pr.head_sha or "") then
    return tools.log_skip(dept, proposal_id, state, "reviewing", "review-result", "skip-foreign(review-result-binding)", "review result does not bind the current linked PR head")
  end
  if fact.decision == "approve" then
    local merge_ready = m_facts.merge_ready_fact(M, comments_for_pr_facts(facts, current_pr), proposal_id, state.version, link.pr_number, current_pr.head_sha)
    if merge_ready == nil then
      return tools.log_skip(dept, proposal_id, state, "reviewing", "merge-ready", "skip-foreign(merge-ready)", "approve review result is not paired with merge-ready marker")
    end
    local payload = payloads_builders.build_devloop_merge_ready_payload(M, proposal_id, link.pr_number, state.version, {
      review_proposal_id = merge_ready.review_proposal_id,
      review_dedup_key = merge_ready.review_dedup_key,
      reviewed_head_sha = merge_ready.head_sha,
      current_head_sha = current_pr.head_sha,
    }, entity_lib.pr_source_ref(issue.repo, link.pr_number))
    M.log_cas_decision(dept, proposal_id, state, "reviewing", "merge-ready", "applied(replay)", "trusted approve review-result fact is visible")
    return tools.raise_effects(dept, proposal_id, "merge-ready", state.version, { add = {}, remove = {} }, {
      { queue = M.pr_package_queue("devloop_merge_ready"), payload = payload },
    })
  end
  local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
  local request = fix_comment_from_feedback(issue, link.pr_number, state.version, {
    proposal_id = proposal_id,
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    reviewed_head_sha = fact.reviewed_head_sha,
    blocking_gap = fact.blocking_gap,
    review_reason = fact.review_reason,
  }, source_ref)
  local effects = {
    { queue = "github-proxy.github_pr_comment_request", payload = request },
  }
  add_issue_label_effect(issue, proposal_id, "fixing", state.version, issue_source_ref(issue), effects, {
    "review-result",
    "label",
    "fixing",
    tostring(proposal_id),
    tostring(state.version),
    tostring(link.pr_number),
  })
  M.log_cas_decision(dept, proposal_id, state, "reviewing", "fixing", "applied(replay)", "trusted reject review-result fact is visible")
  return tools.raise_effects(dept, proposal_id, "fixing", state.version, { add = { "fkst-dev:fixing" }, remove = { "fkst-dev:reviewing" } }, effects)
end

local function review_converge_fact(facts, state, link, current_pr)
  local review_proposal = devloop_base.pr_review_proposal_id(facts.issue.repo, link.pr_number, state.version, current_pr.head_sha)
  local source_ref = entity_lib.pr_source_ref(facts.issue.repo, link.pr_number)
  local records = conv_rounds.review_converge_round_facts(M,
    comments_for_pr_facts(facts, current_pr),
    review_proposal,
    facts.proposal_id,
    state.version,
    current_pr.head_sha,
    convergence_shared.source_ref_digest(source_ref)
  )
  local round = conv_rounds.max_converge_round(M, records)
  local latest = nil
  for _, fact in ipairs(records) do
    if fact.round == round then
      latest = fact
    end
  end
  if latest ~= nil then
    latest.proposal_id = review_proposal
    latest.source_ref = source_ref
    latest.pr_number = link.pr_number
  end
  return latest, records, round
end

local function replay_review_converge(dept, issue, state, facts, tools, link, current_pr)
  local latest, records, round = review_converge_fact(facts, state, link, current_pr)
  if latest == nil then
    return nil
  end
  if conv_rounds.is_true_stall(M, records, round) or round >= config.max_converge_rounds(M) then
    local payload = conv_reconcile.build_devloop_review_reconcile_payload(M, latest, round, facts.proposal_id, state.version, current_pr.head_sha)
    M.log_cas_decision(dept, facts.proposal_id, state, "reviewing", "blocked", "applied(replay)", "trusted review-converge-round fact reached terminal reconcile")
    return tools.raise_effects(dept, facts.proposal_id, "blocked", conv_reconcile.review_reconcile_terminal_state_version(M, state.version, round), { add = { "fkst-dev:blocked" }, remove = { "fkst-dev:reviewing" } }, {
      { queue = "devloop_review_reconcile", payload = payload },
    })
  end
  if review_truth_table_unapproved(latest) then
    local payload = payloads_builders.build_devloop_review_meta_payload(M, latest, facts.proposal_id, state.version, link.pr_number, round, latest.source_ref)
    M.log_cas_decision(dept, facts.proposal_id, state, "reviewing", "review-meta", "applied(replay)", "trusted review-converge-round fact requires review-meta")
    return tools.raise_effects(dept, facts.proposal_id, "review-meta", state.version, { add = { "fkst-dev:review-meta" }, remove = { "fkst-dev:reviewing" } }, {
      { queue = M.pr_package_queue("devloop_review_meta"), payload = payload },
    })
  end
  return nil
end

local function feedback_from_comments(facts, current_pr)
  return facts.fix_feedback or facts.feedback or M.fixing_replay_feedback_fact(comments_for_pr_facts(facts, current_pr), facts.proposal_id, facts.state.version)
end

local function replay_fixing(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link, current_pr, done = linked_open_pr(dept, issue, state, facts, tools, "fixing", "fixing|reviewing")
  if done ~= nil then return done end
  if not M.fixing_version_matches_link(state.version, link.impl_version) then
    return tools.log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(pr-link)", "fixing recovery requires a same-version pr-link marker")
  end
  local feedback = feedback_from_comments(facts, current_pr)
  if feedback == nil then
    return tools.log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(fix-feedback)", "trusted fix feedback marker is not visible")
  end
  if feedback.review_proposal_id == nil or feedback.review_dedup_key == nil or feedback.reviewed_head_sha == nil then
    return tools.log_skip(dept, proposal_id, state, "fixing", "fixing", "skip-foreign(fix-feedback-binding)", "trusted fix feedback marker lacks review binding")
  end
  if tostring(current_pr.head_sha or "") ~= tostring(feedback.reviewed_head_sha or "") then
    local intended_head_sha = git_mechanics.current_branch_head_sha(M.git, link.branch)
    if intended_head_sha ~= nil and tostring(current_pr.head_sha or "") ~= intended_head_sha then
      return tools.log_skip(dept, proposal_id, state, "fixing", "fixing", "skip-stale(head-advanced)", "PR head advanced since rejected review")
    end
    local new_version = M.next_fix_version(state.version)
    local fix = {
      proposal_id = proposal_id,
      pr_number = link.pr_number,
      version = state.version,
      review_proposal_id = feedback.review_proposal_id,
      review_dedup_key = feedback.review_dedup_key,
      reviewed_head_sha = feedback.reviewed_head_sha,
      source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number),
    }
    local effects = {
      {
        queue = "github-proxy.github_pr_comment_request",
        payload = requests_review.build_fix_reviewing_comment_request(M, issue.repo, issue.number, fix, feedback.reviewed_head_sha, current_pr.head_sha, new_version),
      },
    }
    add_issue_label_effect(issue, proposal_id, "reviewing", new_version, issue_source_ref(issue), effects, {
      "fixing",
      "label",
      "reviewing",
      tostring(proposal_id),
      tostring(new_version),
      tostring(link.pr_number),
      tostring(current_pr.head_sha),
    })
    M.log_cas_decision(dept, proposal_id, state, "fixing", "reviewing", "applied(replay)", "push already visible; self-healing missing reviewing marker")
    return tools.raise_effects(dept, proposal_id, "reviewing", new_version, { add = { "fkst-dev:reviewing" }, remove = { "fkst-dev:fixing" } }, effects)
  end
  local payload = payloads_builders.build_replayed_fixing_payload(M, {
    proposal_id = proposal_id,
    impl_version = state.version,
  }, link.pr_number, feedback, entity_lib.pr_source_ref(issue.repo, link.pr_number))
  M.log_cas_decision(dept, proposal_id, state, "fixing", "fixing", "applied(replay)", "trusted fix feedback fact is visible")
  if dept == "observe_pr" then
    local request = fixing_replay_comment_request(issue, link.pr_number, payload, feedback, entity_lib.pr_source_ref(issue.repo, link.pr_number))
    return tools.raise_effects(dept, proposal_id, "fixing", state.version, { add = {}, remove = {} }, {
      { queue = "github-proxy.github_pr_comment_request", payload = request },
    })
  end
  return tools.raise_effects(dept, proposal_id, "fixing", state.version, { add = {}, remove = {} }, {
    { queue = M.pr_package_queue("devloop_fixing"), payload = payload },
  })
end

local function review_meta_decision_fact(facts, current_pr)
  return m_facts.review_meta_decision_fact(M, comments_for_pr_facts(facts, current_pr), facts.proposal_id, facts.state.version)
end

local function replay_review_meta_result(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link, current_pr, done = linked_open_pr(dept, issue, state, facts, tools, "review-meta", "fixing|blocked")
  if done ~= nil then return done end
  local fact = review_meta_decision_fact(facts, current_pr)
  if fact == nil then
    return tools.log_skip(dept, proposal_id, state, "review-meta", "fixing|blocked", "skip-foreign(review-meta)", "trusted review-meta decision marker is not visible")
  end
  if fact.action == "fix" then
    local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
    local feedback = {
      proposal_id = proposal_id,
      review_proposal_id = fact.review_proposal_id,
      review_dedup_key = fact.review_dedup_key,
      reviewed_head_sha = fact.reviewed_head_sha or current_pr.head_sha,
      blocking_gap = fact.blocking_gap,
      review_reason = fact.review_reason,
    }
    local request = fix_comment_from_feedback(issue, link.pr_number, fact.version, feedback, source_ref)
    local effects = {
      { queue = "github-proxy.github_pr_comment_request", payload = request },
    }
    add_issue_label_effect(issue, proposal_id, "fixing", fact.version, issue_source_ref(issue), effects, {
      "review-meta",
      "label",
      "fixing",
      tostring(proposal_id),
      tostring(fact.version),
      tostring(link.pr_number),
    })
    M.log_cas_decision(dept, proposal_id, state, "review-meta", "fixing", "applied(replay)", "trusted review-meta fix decision fact is visible")
    return tools.raise_effects(dept, proposal_id, "fixing", fact.version, { add = { "fkst-dev:fixing" }, remove = { "fkst-dev:review-meta" } }, effects)
  end
  local label_key = base_ids.dedup_key({
    "review-meta",
    "label",
    "blocked",
    tostring(proposal_id),
    tostring(fact.version),
    tostring(link.pr_number),
  })
  local effects = {}
  append_issue_label_effect(issue, proposal_id, "blocked", fact.version, issue_source_ref(issue), effects, label_key)
  M.log_cas_decision(dept, proposal_id, state, "review-meta", "blocked", "applied(replay)", "trusted review-meta block decision fact is visible")
  return tools.raise_effects(dept, proposal_id, "blocked", fact.version, { add = { "fkst-dev:blocked" }, remove = { "fkst-dev:review-meta" } }, effects)
end

local function merge_ready_marker_fact(facts, current_pr)
  return facts.merge_ready or facts["merge-ready"] or m_facts.merge_ready_fact(M, comments_for_pr_facts(facts, current_pr), facts.proposal_id, facts.state.version, facts.link.pr_number, current_pr.head_sha)
end

local function any_merge_ready_marker_fact(facts, current_pr)
  return facts.merge_ready or facts["merge-ready"] or m_facts.merge_ready_fact(M, comments_for_pr_facts(facts, current_pr), facts.proposal_id, facts.state.version, facts.link.pr_number, nil)
end

local function replay_merge_ready_state(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link, current_pr, done = linked_open_pr(dept, issue, state, facts, tools, "merge-ready", "merging|blocked")
  if done ~= nil then return done end
  local comments = comments_for_pr_facts(facts, current_pr)
  local fact = merge_ready_marker_fact(facts, current_pr)
  local lineage_fact = fact or any_merge_ready_marker_fact(facts, current_pr)
  if lineage_fact == nil then
    return tools.log_skip(dept, proposal_id, state, "merge-ready", "merging|blocked", "skip-foreign(merge-ready)", "head-bound merge-ready marker is not visible")
  end
  if fact == nil then
    local carry, carry_reason = M.approved_lineage_carry_over(
      issue.repo,
      link.pr_number,
      proposal_id,
      state.version,
      comments,
      link.base_branch,
      current_pr.head_sha
    )
    if carry_reason == "missing-review-result-approve" then
      M.log_cas_decision(dept, proposal_id, state, "merge-ready", "blocked", "applied(replay)", "merge-ready marker lacks trusted approve review-result")
      return tools.raise_effects(dept, proposal_id, "blocked", state.version, { add = { "fkst-dev:blocked" }, remove = { "fkst-dev:merge-ready" } }, {})
    end
    if carry ~= nil then
      local request = requests_review.build_review_carry_over_comment_request(M, issue.repo, link.pr_number, proposal_id, state.version, carry, entity_lib.pr_source_ref(issue.repo, link.pr_number))
      M.log_cas_decision(dept, proposal_id, state, "merge-ready", "merge-ready", "applied(review-carry-over)", "approved head is ancestor and resolution delta is empty")
      return tools.raise_effects(dept, proposal_id, "merge-ready", state.version, { add = {}, remove = {} }, {
        { queue = "github-proxy.github_pr_comment_request", payload = request },
      })
    end
    return raise_reviewing_for_current_head(dept, issue, state, proposal_id, link, current_pr, lineage_fact.head_sha, "applied(replay)", tostring(carry_reason or "approval-stale"), tools)
  end
  local approved = {
    proposal_id = proposal_id,
    pr_number = link.pr_number,
    version = state.version,
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    reviewed_head_sha = fact.head_sha,
  }
  local approved_ok = m_facts.review_result_approval_matches_event(M, comments, approved)
  if not approved_ok then
    M.log_cas_decision(dept, proposal_id, state, "merge-ready", "blocked", "applied(replay)", "merge-ready marker lacks trusted approve review-result")
    return tools.raise_effects(dept, proposal_id, "blocked", state.version, { add = { "fkst-dev:blocked" }, remove = { "fkst-dev:merge-ready" } }, {})
  end
  local payload = payloads_builders.build_devloop_merge_ready_payload(M, proposal_id, link.pr_number, state.version, {
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    reviewed_head_sha = fact.head_sha,
    current_head_sha = current_pr.head_sha,
  }, entity_lib.pr_source_ref(issue.repo, link.pr_number))
  M.log_cas_decision(dept, proposal_id, state, "merge-ready", "merging", "applied(replay)", "trusted head-bound merge-ready fact is visible")
  return tools.raise_effects(dept, proposal_id, "merging", state.version, { add = { "fkst-dev:merging" }, remove = { "fkst-dev:merge-ready" } }, {
    { queue = M.pr_package_queue("devloop_merge_ready"), payload = payload },
  })
end

local function merging_marker_fact(facts, current_pr)
  return facts.merging or m_facts.merging_fact(M, comments_for_pr_facts(facts, current_pr), facts.proposal_id, facts.link.pr_number, facts.state.version, nil)
end

raise_reviewing_for_current_head = function(dept, issue, state, proposal_id, link, current_pr, old_head_sha, outcome, reason, tools)
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return false
  end
  if not forge_validators.is_git_sha(current_pr.head_sha) then
    return false
  end
  local review_version = state.version
  local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
  local request = requests_review.build_merge_head_reviewing_comment_request(M,
    issue.repo,
    issue.number,
    {
      proposal_id = proposal_id,
      pr_number = link.pr_number,
      reviewed_head_sha = old_head_sha,
    },
    state.version,
    current_pr.head_sha,
    review_version,
    source_ref
  )
  local effects = {
    { queue = "github-proxy.github_pr_comment_request", payload = request },
  }
  add_issue_label_effect(issue, proposal_id, "reviewing", review_version, issue_source_ref(issue), effects, {
    "merging",
    "label",
    "reviewing",
    tostring(proposal_id),
    tostring(review_version),
    tostring(link.pr_number),
    tostring(current_pr.head_sha),
  })
  M.log_cas_decision(dept, proposal_id, state, "merging", "reviewing", outcome, reason)
  return tools.raise_effects(dept, proposal_id, "reviewing", review_version, { add = { "fkst-dev:reviewing" }, remove = { "fkst-dev:merging" } }, effects)
end

local function replay_merging_state(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil then
    return tools.log_skip(dept, proposal_id, state, "merging", "merged|reviewing|fixing|blocked", "skip-foreign(pr-link)", "merging recovery requires a pr-link marker")
  end
  local current_pr = tools.find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    return tools.log_skip(dept, proposal_id, state, "merging", "merged|reviewing|fixing|blocked", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local merge_ready = any_merge_ready_marker_fact(facts, current_pr)
  local merging = merging_marker_fact(facts, current_pr)
  if merge_ready == nil then
    return tools.log_skip(dept, proposal_id, state, "merging", "merged|reviewing|fixing|blocked", "skip-foreign(merge-ready)", "trusted merge-ready marker is not visible")
  end
  local state_name = linked_pr_state(current_pr)
  if state_name == "MERGED" then
    return mark_issue_merged_from_linked_pr(dept, issue, state, proposal_id, link, current_pr, tools)
  end
  if state_name ~= "OPEN" then
    return mark_child_closed_unmerged(dept, issue, state, proposal_id, link, tools, "applied(orphaned-pr-closed)", "linked PR is closed; parent awaiting-pr will re-drive implementation from child terminal")
  end
  local authorized_head = merging and merging.head_sha or merge_ready.head_sha
  if not same_linked_head(link, current_pr)
    or tostring(current_pr.head_sha or "") ~= tostring(merge_ready.head_sha or "")
    or tostring(current_pr.head_sha or "") ~= tostring(authorized_head or "") then
    return raise_reviewing_for_current_head(dept, issue, state, proposal_id, link, current_pr, authorized_head, "applied(replay)", "current PR head no longer matches merge authorization", tools)
  end
  local mergeable, mergeable_reason = check_runs.pr_mergeable(current_pr)
  if merging == nil then
    local payload = payloads_builders.build_devloop_merge_ready_payload(M, proposal_id, link.pr_number, state.version, {
      review_proposal_id = merge_ready.review_proposal_id,
      review_dedup_key = merge_ready.review_dedup_key,
      reviewed_head_sha = merge_ready.head_sha,
      current_head_sha = current_pr.head_sha,
    }, entity_lib.pr_source_ref(issue.repo, link.pr_number))
    M.log_cas_decision(dept, proposal_id, state, "merging", "merging", "applied(replay)", "trusted merge-ready marker is visible and merging receiver needs redrive")
    return tools.raise_effects(dept, proposal_id, "merging", state.version, { add = {}, remove = {} }, {
      { queue = M.pr_package_queue("devloop_merge_ready"), payload = payload },
    })
  end
  if not mergeable and check_runs.is_not_mergeable_reason(mergeable_reason) then
    local fix_version = M.fix_version_from_review_version(state.version)
    local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
    local request = requests_review.build_merge_gate_fix_comment_request(M, issue.repo, issue.number, merge_ready, fix_version, mergeable_reason, current_pr.base_ref_oid, source_ref)
    local effects = {
      { queue = "github-proxy.github_pr_comment_request", payload = request },
    }
    add_issue_label_effect(issue, proposal_id, "fixing", fix_version, issue_source_ref(issue), effects, {
      "merging",
      "label",
      "fixing",
      tostring(proposal_id),
      tostring(fix_version),
      tostring(link.pr_number),
    })
    M.log_cas_decision(dept, proposal_id, state, "merging", "fixing", "applied(replay)", mergeable_reason)
    return tools.raise_effects(dept, proposal_id, "fixing", fix_version, { add = { "fkst-dev:fixing" }, remove = { "fkst-dev:merging" } }, effects)
  end
  local ci_green, ci_reason = M.evaluate_ci_status_gate(current_pr, { repo = issue.repo, dept = dept, proposal_id = proposal_id })
  if ci_green then
    local payload = payloads_builders.build_devloop_merge_ready_payload(M, proposal_id, link.pr_number, state.version, {
      review_proposal_id = merge_ready.review_proposal_id,
      review_dedup_key = merge_ready.review_dedup_key,
      reviewed_head_sha = merge_ready.head_sha,
      current_head_sha = current_pr.head_sha,
    }, entity_lib.pr_source_ref(issue.repo, link.pr_number))
    M.log_cas_decision(dept, proposal_id, state, "merging", "merging", "applied(replay)", "trusted merging marker is visible and merge gates are still eligible")
    return tools.raise_effects(dept, proposal_id, "merging", state.version, { add = {}, remove = {} }, {
      { queue = M.pr_package_queue("devloop_merge_ready"), payload = payload },
    })
  end
  if parsers_misc.is_ci_red_reason(M, ci_reason) then
    local fix_version = M.fix_version_from_review_version(state.version)
    local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
    local request = requests_review.build_merge_gate_fix_comment_request(M, issue.repo, issue.number, merge_ready, fix_version, ci_reason, current_pr.base_ref_oid, source_ref)
    local effects = {
      { queue = "github-proxy.github_pr_comment_request", payload = request },
    }
    add_issue_label_effect(issue, proposal_id, "fixing", fix_version, issue_source_ref(issue), effects, {
      "merging",
      "label",
      "fixing",
      tostring(proposal_id),
      tostring(fix_version),
      tostring(link.pr_number),
    })
    M.log_cas_decision(dept, proposal_id, state, "merging", "fixing", "applied(replay)", ci_reason)
    return tools.raise_effects(dept, proposal_id, "fixing", fix_version, { add = { "fkst-dev:fixing" }, remove = { "fkst-dev:merging" } }, effects)
  end
  M.log_cas_decision(dept, proposal_id, state, "merging", "blocked", "applied(replay)", tostring(ci_reason or "merge-gate-blocked"))
  return tools.raise_effects(dept, proposal_id, "blocked", state.version, { add = { "fkst-dev:blocked" }, remove = { "fkst-dev:merging" } }, {})
end

mark_child_closed_unmerged = function(dept, issue, state, proposal_id, link, tools, outcome, reason)
  local version = transition_version.strip_suffixes(state and state.version)
  local pr_number = link and link.pr_number
  local source_ref = pr_number ~= nil and entity_lib.pr_source_ref(issue.repo, pr_number) or issue.source_ref
  local comment_request = entity_lib.build_entity_comment_request({
    kind = "pr",
    repo = issue.repo,
    number = pr_number,
  }, "github-devloop marked delegated PR child closed without merge"
    .. "\n\nReason: " .. tostring(reason or "closed without merge")
    .. "\n\n" .. M.state_marker(proposal_id, "closed-unmerged", version)
    .. "\n" .. "⟦AI:FKST⟧", base_ids.dedup_key({
    "child-pr",
    "closed-unmerged",
    tostring(proposal_id),
    tostring(version),
    tostring(pr_number),
  }), source_ref)
  comment_request.handoff = {
    kind = "github-devloop.closed_unmerged",
    proposal_id = proposal_id,
    pr_number = pr_number,
    version = version,
    source_ref = source_ref,
  }
  M.log_cas_decision(dept, proposal_id, state, state and state.state or "pr-open", "closed-unmerged", outcome, reason)
  local add_labels, remove_labels = M.state_label_changes("closed-unmerged")
  return tools.raise_effects(dept, proposal_id, "closed-unmerged", version, { add = add_labels, remove = remove_labels }, {
    { queue = "github-proxy.github_pr_comment_request", payload = comment_request },
  })
end

mark_issue_merged_from_linked_pr = function(dept, issue, state, proposal_id, link, pr, tools)
  local head_sha = merged_head_sha(pr)
  if head_sha == nil then
    return tools.log_skip(dept, proposal_id, state, state.state, "merged", "skip-foreign(head)", "merged linked PR head sha is missing")
  end
  local merged_body = comment_strings.comment_string(M, "merged_pr_prefix") .. tostring(link.pr_number)
    .. "\n\n" .. M.state_marker(proposal_id, "merged", state.version)
    .. "\n" .. m_builders.merged_marker(M, proposal_id, link.pr_number, state.version, head_sha)
  local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
  local comment_request = entity_lib.build_entity_comment_request({
    kind = "issue",
    repo = issue.repo,
    number = issue.number,
  }, merged_body, base_ids.dedup_key({
    "orphaned-pr",
    "merged",
    tostring(proposal_id),
    tostring(state.version),
    tostring(link.pr_number),
    tostring(head_sha),
  }), issue.source_ref)
  local label_request = requests_labels.build_state_label_request(M,
    issue.repo,
    issue.number,
    "merged",
    base_ids.dedup_key({
      "orphaned-pr",
      "label",
      "merged",
      tostring(proposal_id),
      tostring(state.version),
      tostring(link.pr_number),
      tostring(head_sha),
    }),
    issue.source_ref
  )
  local add_labels, remove_labels = M.state_label_changes("merged")
  M.log_cas_decision(dept, proposal_id, state, state.state, "merged", "applied(linked-pr-merged)", "linked PR is merged; marking issue complete")
  return tools.raise_effects(dept, proposal_id, "merged", state.version, { add = add_labels, remove = remove_labels }, {
    { queue = "github-proxy.github_issue_comment_request", payload = comment_request },
    { queue = "github-proxy.github_issue_label_request", payload = label_request },
  })
end

local function redrive_absent_replacement_pr(dept, issue, state, proposal_id, link, facts, tools)
  if facts.snapshot.absent_prs ~= nil and facts.snapshot.absent_prs[tostring(link.pr_number or "")] == true then
    return mark_child_closed_unmerged(dept, issue, state, proposal_id, link, tools, "applied(orphaned-pr-absent)", "linked PR is absent; parent awaiting-pr will re-drive implementation from child terminal")
  end
  return nil
end

terminal_linked_pr_action = function(dept, issue, state, proposal_id, link, pr, facts, tools)
  if pr == nil then
    return redrive_absent_replacement_pr(dept, issue, state, proposal_id, link, facts, tools)
  end
  local state_name = linked_pr_state(pr)
  if state_name == "MERGED" then
    return mark_issue_merged_from_linked_pr(dept, issue, state, proposal_id, link, pr, tools)
  end
  if state_name ~= "OPEN" then
    return mark_child_closed_unmerged(dept, issue, state, proposal_id, link, tools, "applied(orphaned-pr-closed)", "linked PR is closed; parent awaiting-pr will re-drive implementation from child terminal")
  end
  return nil
end

local function replay_pr_open(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil or transition_version.strip_suffixes(state.version) ~= transition_version.strip_suffixes(link.impl_version) then
    return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(pr-link)", "pr-open replay requires a same-version pr-link marker")
  end
  for _, item in ipairs(facts.snapshot.prs or {}) do
    if tostring(item.number or "") == tostring(link.pr_number or "") then
      local pr = item.current or {}
      local terminal = terminal_linked_pr_action(dept, issue, state, proposal_id, link, pr, facts, tools)
      if terminal ~= nil then return terminal end
      if tostring(pr.head_ref_name or "") ~= tostring(link.branch or "") then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "linked PR head branch does not match pr-link marker")
      end
      if tostring(pr.base_ref_name or "") ~= tostring(link.base_branch or "") then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(base)", "linked PR base branch does not match pr-link marker")
      end
      if not forge_validators.is_git_sha(pr.head_sha) then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "linked PR head sha is missing")
      end
      local mergeable, mergeable_reason = check_runs.pr_mergeable(pr)
      if not mergeable and check_runs.is_not_mergeable_reason(mergeable_reason) then
        local fix_version = M.next_fix_version(state.version)
        local source_ref = entity_lib.pr_source_ref(issue.repo, link.pr_number)
        local review_fact = {
          proposal_id = proposal_id,
          review_proposal_id = devloop_base.pr_review_proposal_id(issue.repo, link.pr_number, state.version, pr.head_sha),
          review_dedup_key = "observe-pr-conflict/" .. tostring(proposal_id) .. "/" .. tostring(state.version) .. "/" .. tostring(link.pr_number),
          reviewed_head_sha = pr.head_sha,
          blocking_gap = mergeable_reason,
          review_reason = mergeable_reason,
        }
        local comment_request = fix_comment_from_feedback(issue, link.pr_number, fix_version, review_fact, source_ref)
        M.log_cas_decision(dept, proposal_id, state, "pr-open", "fixing", "applied(replay)", "linked PR is not mergeable")
        return tools.raise_effects(dept, proposal_id, "fixing", fix_version, { add = { "fkst-dev:fixing" }, remove = { "fkst-dev:pr-open" } }, {
          { queue = "github-proxy.github_pr_comment_request", payload = comment_request },
        })
      end
      local review_version = M.review_redrive_version(state, {
        repo = issue.repo,
        number = link.pr_number,
        head_sha = pr.head_sha,
      })
      local review_proposal_id = devloop_base.pr_review_proposal_id(issue.repo, link.pr_number, review_version, pr.head_sha)
      if m_facts.has_any_review_result_marker(M, facts.snapshot.comments, review_proposal_id, proposal_id) then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-idempotent(review result visible)", "review already produced a result")
      end
      local fields = tools.resolve_payload_fields(row, state, {
        issue = issue,
        state = state,
        link = link,
        proposal_id = proposal_id,
      })
      fields.version = review_version
      local reviewing_comment = requests_review.build_reviewing_comment_request(M, issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref)
      M.log_cas_decision(dept, proposal_id, state, "pr-open", "reviewing", "applied(replay)", "linked PR head/base match pr-link marker")
      return tools.raise_effects(dept, proposal_id, "pr-open", state.version, { add = {}, remove = {} }, {
        { queue = "github-proxy.github_pr_comment_request", payload = reviewing_comment },
      })
    end
  end
  local absent_redrive = redrive_absent_replacement_pr(dept, issue, state, proposal_id, link, facts, tools)
  if absent_redrive ~= nil then return absent_redrive end
  return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
end

local function replay_reviewing(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link, current_pr, done = linked_open_pr(dept, issue, state, facts, tools, "reviewing", "reviewing")
  if done ~= nil then return done end
  local result_replay = replay_review_result(dept, issue, state, facts, tools, link, current_pr)
  if result_replay ~= nil then
    return result_replay
  end
  local converge_replay = replay_review_converge(dept, issue, state, facts, tools, link, current_pr)
  if converge_replay ~= nil then
    return converge_replay
  end
  local review_version = M.review_redrive_version(state, {
    repo = issue.repo,
    number = link.pr_number,
    head_sha = current_pr.head_sha,
  })
  local fields = tools.resolve_payload_fields(row, state, {
    issue = issue,
    state = state,
    link = link,
    proposal_id = proposal_id,
  })
  fields.version = review_version
  local review_proposal_id = devloop_base.pr_review_proposal_id(issue.repo, fields.pr_number, fields.version, current_pr.head_sha)
  if m_facts.has_any_review_result_marker(M, current_pr.comments, review_proposal_id, proposal_id) then
    tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-idempotent(review result visible)", "review already produced a result")
    return true
  end
  M.log_cas_decision(dept, proposal_id, state, "reviewing", "reviewing", "applied(replay)", "current PR head has no trusted review result")
  local effects = {}
  if tostring(fields.version or "") ~= tostring(state.version or "") then
    table.insert(effects, {
      queue = "github-proxy.github_pr_comment_request",
      payload = requests_review.build_reviewing_comment_request(M, issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref),
    })
  elseif dept == "observe_pr" then
    table.insert(effects, {
      queue = "github-proxy.github_pr_comment_request",
      payload = requests_review.build_reviewing_comment_request(M, issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref),
    })
  else
    table.insert(effects, {
      queue = "devloop_reviewing",
      payload = payloads_builders.build_devloop_reviewing_payload(M, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref, fields.version),
    })
  end
  return tools.raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, effects)
end

local function review_replayers(tools)
  local replayers = {}
  replayers["pr-open"] = function(dept, issue, state, row, facts)
    return replay_pr_open(dept, issue, state, row, facts, tools)
  end
  replayers.reviewing = function(dept, issue, state, row, facts)
    return replay_reviewing(dept, issue, state, row, facts, tools)
  end
  replayers.fixing = function(dept, issue, state, row, facts)
    return replay_fixing(dept, issue, state, row, facts, tools)
  end
  replayers["review-meta"] = function(dept, issue, state, row, facts)
    return replay_review_meta_result(dept, issue, state, row, facts, tools)
  end
  replayers["merge-ready"] = function(dept, issue, state, row, facts)
    return replay_merge_ready_state(dept, issue, state, row, facts, tools)
  end
  replayers.merging = function(dept, issue, state, row, facts)
    return replay_merging_state(dept, issue, state, row, facts, tools)
  end
  tools.terminal_linked_pr_action = function(dept, issue, state, proposal_id, link, pr, facts)
    return terminal_linked_pr_action(dept, issue, state, proposal_id, link, pr, facts, tools)
  end
  M.terminal_linked_pr_action = tools.terminal_linked_pr_action
  return replayers
end

return review_replayers
end

return S
