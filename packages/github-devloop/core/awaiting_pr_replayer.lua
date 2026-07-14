local git_mechanics = require("devloop.git_mechanics")
local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local requests_labels = require("devloop.requests.labels")
local parsers_pr = require("devloop.parsers.pr")
local config = require("devloop.config")
local m_facts = require("devloop.markers.facts")
local devloop_state = require("devloop.state")
local devloop_commands = require("devloop.commands")
local git_commands = require("devloop.commands.git_ops")
local pr_commands = require("devloop.commands.prs")
-- `awaiting-pr` is the issue-side `dependency_wait` twin: poll-reconcile the delegated PR's terminal fact and never drive `github-devloop-pr` internal lifecycle queues; the PR package owns those queues.
local S, replay_fields = {}, require("devloop.replay_fields")
local replayer = require("devloop.replayer")
local forge_validators = require("devloop.forge_validators")
local contract_time = require("contract.time")
local contract_strings = require("contract.strings")
local transition_version = require("contract.transition_version")
local autonomy_ledger = require("devloop.autonomy_ledger")
local m_builders = require("devloop.markers.builders")
local devloop_entity_view = require("devloop.github_proxy_entity_view")
local devloop_logging = require("devloop.logging")

function S.fetch_then_scan_rollup_receipts(candidates, fetch_receipt, receipt_contains_child)
  local receipt_heads = {}
  for _, candidate in ipairs(candidates) do
    local receipt_head = fetch_receipt(candidate)
    if receipt_head ~= nil then
      table.insert(receipt_heads, receipt_head)
    end
  end
  for _, receipt_head in ipairs(receipt_heads) do
    if receipt_contains_child(receipt_head) then
      return true
    end
  end
  return false
end

function S.install(M)
local child_terminal_states = {
  merged = true,
  ["closed-unmerged"] = true,
  blocked = true,
}
local canonical_pr_is_merged, origin_matches_delegation, canonical_merged_child_state, merged_child_landed_on_upstream
local function log_skip(dept, proposal_id, state, from_state, to_state, outcome, reason)
  return replayer.replay_log_decline(M, "stuck", dept, proposal_id, state, from_state, to_state, outcome, reason)
end

local function raise_effects(dept, proposal_id, apply_state, version, label_changes, effects)
  return replay_fields.replay_raise_effects(devloop_logging.log_apply, devloop_logging.log_raise, dept, proposal_id, apply_state, version, label_changes, effects)
end

local function next_reimplementation_version(version)
  return transition_version.reimplement_at(M.implementation_base_version(version), 1)
end

local function closed_unmerged_generation(issue, state, current_pr)
  local root_version = M.implementation_base_version(state.version)
  local original_branch = devloop_base.implement_branch(issue.repo, issue.number, root_version)
  local replacement_version = transition_version.reimplement_at(root_version, 1)
  local replacement_branch = devloop_base.implement_branch(issue.repo, issue.number, replacement_version)
  local current_branch = tostring(current_pr and current_pr.head_ref_name or "")
  if current_branch == original_branch then
    return "original"
  end
  if current_branch == replacement_branch then
    return "replacement"
  end
  return nil
end

local function parent_state_for_child_terminal(state, child_state, generation)
  if child_state.state == "merged" then
    return {
      to_state = "merged",
      version = state.version,
      reason = "child-pr-merged",
    }
  end
  if child_state.state == "closed-unmerged" then
    if generation == "replacement" then
      return {
        to_state = "blocked",
        version = tostring(state.version or "") .. "/blocked/replacement-budget-exhausted",
        reason = "replacement-budget-exhausted",
      }
    end
    return {
      to_state = "ready",
      version = next_reimplementation_version(state.version),
      reason = "child-pr-closed-unmerged",
    }
  end
  return {
    to_state = "blocked",
    version = tostring(state.version or "") .. "/blocked/child-pr-blocked",
    reason = "child-pr-blocked",
  }
end

local function read_delegated_child_pr(dept, issue, delegation)
  local pr_view = devloop_entity_view.fetch_pr_view_origin(issue.repo, delegation.pr_number, nil, {
    force_fresh = true,
    consumer = dept,
  })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: awaiting-pr-child-view-failed: " .. tostring(pr_view.stderr))
  end
  local current_pr = parsers_pr.parse_pr_view_origin(pr_view.stdout)
  current_pr.number, current_pr.force_fresh = delegation.pr_number, true
  return current_pr
end

local function version_is_same_lineage_or_descendant(version, lineage)
  local candidate = tostring(version or "")
  local base = tostring(lineage or "")
  if transition_version.strip_suffixes(candidate) == transition_version.strip_suffixes(base) then
    return true
  end
  return base ~= "" and (candidate == base or candidate:sub(1, #base + 1) == base .. "/")
end

function M.delegation_identity_matches(left, right)
  if type(left) ~= "table" or type(right) ~= "table"
    or tostring(left.pr_number or "") ~= tostring(right.pr_number or "") then
    return false
  end
  local left_version = tostring(left.version or "")
  local right_version = tostring(right.version or "")
  if left_version == "" or right_version == "" then
    return false
  end
  return version_is_same_lineage_or_descendant(left_version, right_version)
    or version_is_same_lineage_or_descendant(right_version, left_version)
end

local function child_lineage_matches_delegation(state, delegation, child_state)
  return tostring(delegation.version or "") == tostring(state.version or "")
    and version_is_same_lineage_or_descendant(child_state.version, delegation.version)
end

local function autonomy_post_merge_pr(pr)
  if type(pr) ~= "table" or type(pr.status_check_rollup) ~= "table" or #pr.status_check_rollup == 0 then
    return nil
  end
  return pr
end

local function resume_terminal_markers(issue, next_state, delegation, current_pr)
  if next_state.to_state ~= "merged" then
    return ""
  end
  local head_sha = tostring(current_pr and current_pr.head_sha or "")
  if not forge_validators.is_git_sha(head_sha) then
    error("github-devloop: avm-ledger-missing-head-sha: awaiting-pr autonomy result requires merged PR head sha")
  end
  local merge_ready = {
    proposal_id = delegation.proposal_id,
    pr_number = delegation.pr_number,
    version = next_state.version,
    reviewed_head_sha = head_sha,
  }
  local autonomy_record = autonomy_ledger.autonomy_result_record(M, issue.repo, issue.number, merge_ready, issue, autonomy_post_merge_pr(current_pr))
  return "\n" .. m_builders.merged_marker(M, delegation.proposal_id, delegation.pr_number, next_state.version, head_sha, autonomy_record)
    .. "\n" .. autonomy_ledger.autonomy_result_marker(autonomy_record)
end

local function build_resume_comment_request(issue, state, next_state, child_state, delegation, current_pr)
  local source_ref = issue.source_ref or entity_lib.issue_source_ref(issue.repo, issue.number)
  local state_marker = devloop_state.state_marker(delegation.proposal_id, next_state.to_state, next_state.version)
  local request = entity_lib.build_entity_comment_request({
    kind = "issue",
    repo = issue.repo,
    number = issue.number,
  }, "github-devloop resumed parent issue from delegated PR child state"
    .. "\n\nChild PR: #" .. tostring(delegation.pr_number)
    .. "\nChild state: " .. tostring(child_state.state)
    .. "\nReason: " .. tostring(next_state.reason)
    .. "\n\n" .. state_marker
    .. resume_terminal_markers(issue, next_state, delegation, current_pr), base_ids.dedup_key({
    "awaiting-pr",
    "resume",
    tostring(delegation.proposal_id),
    tostring(state.version),
    tostring(delegation.pr_number),
    tostring(delegation.delegation),
    tostring(child_state.state),
    tostring(next_state.to_state),
    tostring(next_state.version),
  }), source_ref)
  if next_state.to_state == "ready" then
    request.handoff = {
      kind = "github-devloop.ready",
      proposal_id = delegation.proposal_id,
      version = next_state.version,
      marker_version = next_state.version,
      source_ref = source_ref,
    }
  end
  return request
end

local function build_awaiting_pr_canonicalization_comment_request(issue, state, delegation)
  local source_ref = issue.source_ref or entity_lib.issue_source_ref(issue.repo, issue.number)
  local child_proposal = delegation.pr_proposal_id or delegation.pr_proposal
  local body = "github-devloop canonicalized delegated PR handoff after child merge"
    .. "\n\nDelegated PR: #" .. tostring(delegation.pr_number)
    .. "\n\n" .. devloop_state.state_marker(delegation.proposal_id, "awaiting-pr", state.version)
    .. "\n" .. m_builders.pr_delegation_marker(delegation.proposal_id,
      child_proposal,
      delegation.pr_number,
      state.version,
      delegation.delegation
    )
  return entity_lib.build_entity_comment_request({
    kind = "issue",
    repo = issue.repo,
    number = issue.number,
  }, body, base_ids.dedup_key({
    "awaiting-pr",
    "canonicalize",
    "implementing",
    tostring(delegation.proposal_id),
    tostring(state.version),
    tostring(delegation.pr_number),
    tostring(delegation.delegation),
  }), source_ref)
end

function M.implementing_to_awaiting_pr_transition_status(state)
  return devloop_state.versioned_transition_status(state, { "implementing" }, "awaiting-pr", state and state.version)
end

function M.canonicalize_implementing_merged_delegated_pr(dept, issue, state, facts)
  local proposal_id = facts.proposal_id
  local transition = M.implementing_to_awaiting_pr_transition_status(state)
  if transition ~= "apply" and transition ~= "idempotent" then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", devloop_state.cas_outcome(state, transition, state and state.version), "merged delegated PR canonicalization requires implementing state")
  end
  local delegation = facts["pr-delegation"] or facts.pr_delegation
  if delegation == nil then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", "skip-foreign(pr-delegation-missing)", "implementing parent has no visible delegated PR marker")
  end
  if tostring(delegation.proposal_id or "") ~= tostring(proposal_id or "")
    or tostring(delegation.version or "") ~= tostring(state.version or "") then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", "skip-stale(pr-delegation-version)", "pr-delegation proposal or version does not match implementing state")
  end
  local pr_repo, pr_number = entity_lib.parse_pr_proposal_id(delegation.pr_proposal_id or delegation.pr_proposal)
  if pr_repo ~= issue.repo or tostring(pr_number or "") ~= tostring(delegation.pr_number or "") then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", "skip-stale(pr-delegation-child)", "pr-delegation child identity is malformed or cross-repo")
  end
  local current_pr = facts.current_pr
  if type(current_pr) ~= "table" or current_pr.force_fresh ~= true then
    current_pr = read_delegated_child_pr(dept, issue, delegation)
  end
  local canonical_merged_state = canonical_merged_child_state(issue, state, delegation, current_pr)
  if canonical_merged_state == nil then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", "skip-pending(canonical-child-pr-merged-missing)", "delegated child PR is not canonically merged by GitHub")
  end
  if transition == "idempotent" then
    return log_skip(dept, proposal_id, state, "implementing", "awaiting-pr", "skip-idempotent(already at to_state)", "parent issue already has awaiting-pr marker")
  end

  local source_ref = issue.source_ref or entity_lib.issue_source_ref(issue.repo, issue.number)
  local comment_request = build_awaiting_pr_canonicalization_comment_request(issue, state, delegation)
  local label_request = requests_labels.build_state_label_request(issue.repo,
    issue.number,
    "awaiting-pr",
    base_ids.dedup_key({
      "awaiting-pr",
      "canonicalize",
      "implementing",
      "label",
      tostring(proposal_id),
      tostring(state.version),
      tostring(delegation.pr_number),
      tostring(delegation.delegation),
    }),
    source_ref
  )
  local add_labels, remove_labels = devloop_state.state_label_changes("awaiting-pr")
  devloop_logging.log_cas_decision(dept, proposal_id, state, "implementing", "awaiting-pr", "applied(merged-delegated-pr-canonicalized)", "canonical merged PR child made missing parent handoff visible")
  return raise_effects(dept, proposal_id, "awaiting-pr", state.version, { add = add_labels, remove = remove_labels }, {
    { queue = "github-proxy.github_issue_comment_request", payload = comment_request },
    { queue = "github-proxy.github_issue_label_request", payload = label_request },
  })
end

function M.close_canonically_merged_delegated_issue(dept, issue, state, facts)
  local proposal_id = facts.proposal_id
  local delegation = facts["pr-delegation"] or facts.pr_delegation
  if delegation == nil then
    return false, nil
  end
  if tostring(delegation.proposal_id or "") ~= tostring(proposal_id or "")
    or not version_is_same_lineage_or_descendant(state and state.version, delegation.version) then
    log_skip(dept, proposal_id, state, tostring(state and state.state or "unknown"), "closed", "skip-stale(pr-delegation-version)", "canonical merged issue close requires the current delegation lineage")
    return false, nil
  end
  local pr_repo, pr_number = entity_lib.parse_pr_proposal_id(delegation.pr_proposal_id or delegation.pr_proposal)
  if pr_repo ~= issue.repo or tostring(pr_number or "") ~= tostring(delegation.pr_number or "") then
    log_skip(dept, proposal_id, state, tostring(state and state.state or "unknown"), "closed", "skip-stale(pr-delegation-child)", "canonical merged issue close requires a same-repository delegated PR")
    return false, nil
  end
  local current_pr = facts.current_pr
  if type(current_pr) ~= "table" or current_pr.force_fresh ~= true then
    current_pr = read_delegated_child_pr(dept, issue, delegation)
  end
  if canonical_merged_child_state(issue, state, delegation, current_pr) == nil then
    return false, current_pr
  end
  local landed, outcome, reason = merged_child_landed_on_upstream(dept, issue, state, delegation, current_pr)
  if not landed then
    log_skip(dept, proposal_id, state, tostring(state and state.state or "unknown"), "closed", outcome, reason)
    return false, current_pr
  end
  if config.write_mode() ~= "real" then
    log_skip(dept, proposal_id, state, tostring(state and state.state or "unknown"), "closed", "skip-dry-run", "canonical merged delegated issue would close in real write mode")
    return false, current_pr
  end
  local close_result = devloop_commands.gh_issue_close(issue.repo, issue.number, 60)
  if close_result.exit_code ~= 0 then
    error("github-devloop: canonical-merged-issue-close-failed: " .. tostring(close_result.stderr))
  end
  devloop_entity_view.invalidate_entity_after_write(issue.repo, "issue", issue.number)
  devloop_logging.log_cas_decision(dept, proposal_id, state, tostring(state and state.state or "unknown"), "closed", "applied(canonical-child-pr-merged)", "canonical delegated PR merge is landed on the configured upstream branch")
  return true, current_pr
end

function M.replay_awaiting_pr_state(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  if state.state ~= "awaiting-pr" then
    return log_skip(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-foreign(state)", "awaiting-pr replay requires awaiting-pr state")
  end
  local delegation = facts["pr-delegation"] or facts.pr_delegation
  if delegation == nil then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-foreign(pr-delegation-missing)", "awaiting-pr marker is visible without matching pr-delegation")
  end
  if tostring(delegation.proposal_id or "") ~= tostring(proposal_id or "")
    or tostring(delegation.version or "") ~= tostring(state.version or "") then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(pr-delegation-version)", "pr-delegation proposal or version does not match awaiting-pr state")
  end
  local pr_repo, pr_number = entity_lib.parse_pr_proposal_id(delegation.pr_proposal_id or delegation.pr_proposal)
  if pr_repo ~= issue.repo or tostring(pr_number or "") ~= tostring(delegation.pr_number or "") then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(pr-delegation-child)", "pr-delegation child identity is malformed or cross-repo")
  end
  local current_pr = (facts.current_pr ~= nil and facts.current_pr.force_fresh == true) and facts.current_pr or read_delegated_child_pr(dept, issue, delegation)
  local child_state = facts.child_state or facts["child-state"] or require("devloop.entity").current_entity_state(current_pr.comments, delegation.proposal_id)
  local canonical_merged_state = canonical_merged_child_state(issue, state, delegation, current_pr)
  if canonical_merged_state ~= nil then
    child_state = canonical_merged_state
  end
  if child_state == nil or child_state.state == nil then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-pending(child-terminal-missing)", "delegated child PR has no trusted terminal marker or canonical merged state")
  end
  if child_terminal_states[child_state.state] ~= true then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-pending(child-nonterminal)", "delegated child PR is not terminal")
  end
  if not child_lineage_matches_delegation(state, delegation, child_state) then
    return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(child-state-lineage)", "child terminal state does not match parent delegation lineage")
  end
  local generation = nil
  local child_closed_unmerged = devloop_state.reached(
    current_pr.comments,
    delegation.proposal_id,
    "closed-unmerged",
    { domain = "github-devloop-pr", lineage_base = state.version }
  ) and not devloop_state.reached(
    current_pr.comments,
    delegation.proposal_id,
    "merged",
    { domain = "github-devloop-pr", lineage_base = state.version }
  )
  if child_closed_unmerged then
    generation = closed_unmerged_generation(issue, state, current_pr)
    if generation == nil then
      return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-stale(child-branch-lineage)", "closed child PR is not on a deterministic original or replacement implementation branch")
    end
  end
  local next_state = parent_state_for_child_terminal(state, child_state, generation)
  if next_state.to_state == "merged" then
    if canonical_merged_state == nil then
      return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", "skip-pending(canonical-child-pr-merged-missing)", "delegated child PR has a merged marker but is not canonically merged by GitHub")
    end
    local landed, outcome, reason = merged_child_landed_on_upstream(dept, issue, state, delegation, current_pr)
    if not landed then
      return log_skip(dept, proposal_id, state, "awaiting-pr", "awaiting-pr", outcome, reason)
    end
  end
  local transition = devloop_state.versioned_transition_status(state, { "awaiting-pr" }, next_state.to_state, state.version)
  if transition ~= "apply" and transition ~= "idempotent" then
    return log_skip(dept, proposal_id, state, "awaiting-pr", next_state.to_state, devloop_state.cas_outcome(state, transition, state.version), next_state.reason)
  end
  if transition == "idempotent" then
    return log_skip(dept, proposal_id, state, "awaiting-pr", next_state.to_state, "skip-idempotent(already at to_state)", "parent issue already reflects delegated child terminal")
  end
  if devloop_state.has_state_marker(issue.comments, proposal_id, next_state.to_state, next_state.version) then
    return log_skip(dept, proposal_id, state, "awaiting-pr", next_state.to_state, "skip-idempotent(target marker already visible)", "parent issue already has the exact delegated child terminal marker")
  end

  local comment_request = build_resume_comment_request(issue, state, next_state, child_state, delegation, current_pr)
  local label_request = requests_labels.build_state_label_request(issue.repo,
    issue.number,
    next_state.to_state,
    base_ids.dedup_key({
      "awaiting-pr",
      "label",
      tostring(proposal_id),
      tostring(delegation.pr_number),
      tostring(delegation.delegation),
      tostring(next_state.to_state),
      tostring(next_state.version),
    }),
    issue.source_ref or entity_lib.issue_source_ref(issue.repo, issue.number)
  )
  local add_labels, remove_labels = devloop_state.state_label_changes(next_state.to_state)
  devloop_logging.log_cas_decision(dept, proposal_id, state, "awaiting-pr", next_state.to_state, "applied(" .. next_state.reason .. ")", "delegated child terminal fact matched parent delegation")
  local effects = {
    { queue = "github-proxy.github_issue_comment_request", payload = comment_request },
    { queue = "github-proxy.github_issue_label_request", payload = label_request },
  }
  if next_state.to_state == "merged" and config.write_mode() == "real" then
    local close_result = devloop_commands.gh_issue_close(issue.repo, issue.number, 60)
    if close_result.exit_code ~= 0 then
      error("github-devloop: awaiting-pr-issue-close-failed: " .. tostring(close_result.stderr))
    end
    devloop_entity_view.invalidate_entity_after_write(issue.repo, "issue", issue.number)
  end
  return raise_effects(dept, proposal_id, next_state.to_state, next_state.version, { add = add_labels, remove = remove_labels }, effects)
end

canonical_pr_is_merged = function(current_pr)
  local state = tostring(current_pr and current_pr.state or ""):upper()
  if state == "MERGED" then
    return true
  end
  local merged_at = current_pr and current_pr.merged_at
  if type(merged_at) ~= "string" then
    return false
  end
  return contract_time.iso_timestamp_epoch_seconds(merged_at) ~= nil
end

origin_matches_delegation = function(issue, delegation, current_pr, branches)
  local origin = m_facts.pr_origin_fact(current_pr and current_pr.comments)
  if origin == nil
    or origin.pr_native == true
    or tostring(origin.proposal_id or "") ~= tostring(delegation.proposal_id or "")
    or tostring(origin.issue_number or "") ~= tostring(issue.number or "")
    or transition_version.strip_suffixes(origin.impl_version) ~= transition_version.strip_suffixes(delegation.version)
    or tostring(origin.branch or "") ~= tostring(current_pr and current_pr.head_ref_name or "")
    or tostring(origin.base_branch or "") ~= tostring(current_pr and current_pr.base_ref_name or "") then
    return false
  end
  if branches ~= nil and tostring(origin.base_branch or "") ~= tostring(branches.integration or "") then
    return false
  end
  return true
end

canonical_merged_child_state = function(issue, state, delegation, current_pr)
  if not canonical_pr_is_merged(current_pr) then
    return nil
  end
  if not origin_matches_delegation(issue, delegation, current_pr) then
    return nil
  end
  return {
    state = "merged",
    version = delegation.version or state.version,
    proposal_id = delegation.proposal_id,
    head_sha = current_pr.head_sha,
    merge_commit_sha = current_pr.merge_commit_sha,
  }
end

merged_child_landed_on_upstream = function(dept, issue, state, delegation, current_pr)
  local branches = config.branch_config()
  if not origin_matches_delegation(issue, delegation, current_pr, branches) then
    return false, "skip-stale(pr-origin-rollup-lineage)", "merged child PR lacks current split-topology origin facts"
  end
  if tostring(branches.integration or "") == tostring(branches.upstream or "") then
    return true
  end
  local merge_commit_sha = tostring(current_pr and current_pr.merge_commit_sha or "")
  if not forge_validators.is_git_sha(merge_commit_sha) then
    return false, "skip-pending(merge-commit-missing)", "canonical merged child PR has no GitHub mergeCommit.oid"
  end
  local listed = pr_commands.gh_pr_list_promotions(
    issue.repo,
    branches.integration,
    branches.upstream,
    60
  )
  if listed.exit_code ~= 0 then
    error("github-devloop: awaiting-pr-rollup-receipt-list-failed: " .. tostring(listed.stderr))
  end
  local candidates = parsers_pr.parse_pr_list_promotions(listed.stdout)
  local landed = git_mechanics.with_repo_ref_store_lock(issue.repo, function()
    return S.fetch_then_scan_rollup_receipts(candidates, function(candidate)
      local branch_match = tostring(candidate.head_ref_name or "") == tostring(branches.integration or "")
        and tostring(candidate.base_ref_name or "") == tostring(branches.upstream or "")
      local canonically_merged = contract_time.iso_timestamp_epoch_seconds(candidate.merged_at) ~= nil
      if branch_match and canonically_merged then
        if tostring(candidate.head_repository or "") == ""
          or not forge_validators.is_git_sha(candidate.head_sha)
          or tonumber(candidate.number) == nil then
          error("github-devloop: awaiting-pr-rollup-receipt-invalid: merged rollup PR metadata is incomplete")
        end
        if tostring(candidate.head_repository) == tostring(issue.repo) then
          git_mechanics.run_required(
            git_commands.git_fetch_pr_head_ref("origin", candidate.number, 60),
            "awaiting-pr rollup receipt fetch"
          )
          local fetched = git_mechanics.run_required(
            git_commands.git_fetch_head_commit(30),
            "awaiting-pr rollup receipt head"
          )
          local fetched_head = contract_strings.trim(fetched.stdout)
          if fetched_head ~= tostring(candidate.head_sha) then
            error("github-devloop: awaiting-pr-rollup-receipt-head-mismatch: fetched rollup PR head differs from GitHub metadata")
          end
          return fetched_head
        end
      end
      return nil
    end, function(receipt_head)
      return git_mechanics.is_ancestor(M.git, merge_commit_sha, receipt_head, "awaiting-pr rollup receipt ancestry")
    end)
  end)
  if not landed then
    return false, "skip-pending(rollup-receipt-missing)", "no merged rollup PR into upstream preserves the child PR merge commit"
  end
  return true
end

return {
  ["awaiting-pr"] = M.replay_awaiting_pr_state,
  implementing_to_awaiting_pr_transition_status = M.implementing_to_awaiting_pr_transition_status,
  canonicalize_implementing_merged_delegated_pr = M.canonicalize_implementing_merged_delegated_pr,
  close_canonically_merged_delegated_issue = M.close_canonically_merged_delegated_issue,
  delegation_identity_matches = M.delegation_identity_matches,
}
end

return S
