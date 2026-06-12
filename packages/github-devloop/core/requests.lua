local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"
local display_separator = " — "
local max_display_question_len = 2000
local max_display_digest_len = 600
local max_display_attr_len = 120
local max_display_block_len = 5000
local max_verdict_summary_items = 8
local max_verdict_summary_len = 600

local function bounded_neutralized_text(value, limit)
  local text = tostring(value or "")
  local cap = limit or max_display_digest_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  text = M.neutralize_untrusted_comment_text(text)
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

local function angle_display_text(item)
  if type(item) ~= "table" then
    return nil
  end
  local angle = bounded_neutralized_text(item.angle or "unknown", max_display_attr_len)
  local verdict = bounded_neutralized_text(item.verdict or "invalid", max_display_attr_len)
  local digest = item.digest
  if digest == nil or tostring(digest) == "" then
    digest = item.reply
  end
  digest = bounded_neutralized_text(digest or "", max_display_digest_len)
  if digest == "" then
    return "- " .. angle .. ": " .. verdict
  end
  return "- " .. angle .. ": " .. verdict .. display_separator .. digest
end

local function build_convergence_display(header, unresolved, round)
  local lines = {
    header .. tostring(round) .. M.comment_string("convergence_suffix"),
  }
  local question = bounded_neutralized_text(unresolved and unresolved.narrowed_question or "", max_display_question_len)
  if question ~= "" then
    table.insert(lines, "")
    table.insert(lines, M.comment_string("narrowed_question_label") .. question)
  end
  local angle_lines = {}
  if type(unresolved) == "table" and type(unresolved.angle_digests) == "table" then
    for _, item in ipairs(unresolved.angle_digests) do
      local line = angle_display_text(item)
      if line ~= nil then
        table.insert(angle_lines, line)
      end
    end
  end
  if #angle_lines > 0 then
    table.insert(lines, "")
    table.insert(lines, M.comment_string("angle_stances_label"))
    for _, line in ipairs(angle_lines) do
      table.insert(lines, line)
    end
  end
  local body = table.concat(lines, "\n")
  if #body > max_display_block_len then
    body = M.truncate_utf8(body, max_display_block_len)
  end
  return body
end

local function build_verdict_summary(angle_results)
  if type(angle_results) ~= "table" then
    return nil
  end
  local parts = {}
  for _, item in ipairs(angle_results) do
    if #parts >= max_verdict_summary_items then
      break
    end
    if type(item) == "table" then
      local angle = bounded_neutralized_text(item.angle or "unknown", max_display_attr_len)
      local verdict = bounded_neutralized_text(item.verdict or "invalid", max_display_attr_len)
      table.insert(parts, angle .. "=" .. verdict)
    end
  end
  if #parts == 0 then
    return nil
  end
  local summary = M.comment_string("verdict_summary_label") .. table.concat(parts, " ")
  if #summary > max_verdict_summary_len then
    summary = M.truncate_utf8(summary, max_verdict_summary_len)
  end
  return summary
end

local function bounded_blocking_gap(M, reached)
  local gap = reached and reached.blocking_gap
  if gap == nil and type(reached and reached.blocking_gaps) == "table" then
    gap = reached.blocking_gaps[1]
  end
  local text = tostring(gap or ""):gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  if #text > M._max_blocking_gap_len then
    text = M.truncate_utf8(text, M._max_blocking_gap_len)
  end
  return text
end

function M.build_label_request(repo, issue_number, add_labels, remove_labels, dedup_key, source_ref)
  return {
    schema = "github-proxy.label.v1",
    repo = repo,
    issue_number = issue_number,
    add_labels = add_labels or {},
    remove_labels = remove_labels or {},
    dedup_key = dedup_key,
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_state_label_request(repo, issue_number, to_state, dedup_key_value, source_ref)
  local add_labels, remove_labels = M.state_label_changes(to_state)
  return M.build_label_request(repo, issue_number, add_labels, remove_labels, dedup_key_value, source_ref)
end

function M.build_thinking_label_request(issue, proposal)
  return M.build_state_label_request(
    issue.repo,
    issue.number,
    "thinking",
    proposal.dedup_key .. "/label/thinking",
    issue.source_ref
  )
end

function M.build_observe_comment_request(issue, proposal)
  return {
    schema = "github-proxy.v1",
    repo = issue.repo,
    issue_number = issue.number,
    body = M.comment_string("thinking_started") .. "\n\n"
      .. M.state_marker(proposal.proposal_id, "thinking", proposal.dedup_key),
    dedup_key = M._dedup_key({
      tostring(proposal.proposal_id),
      "comment",
      "thinking",
      tostring(proposal.dedup_key),
    }),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_result_label_request(repo, issue_number, reached)
  return M.build_state_label_request(
    repo,
    issue_number,
    "ready",
    tostring(reached.proposal_id) .. "/label/" .. tostring(reached.decision),
    reached.source_ref
  )
end

function M.build_result_comment_request(repo, issue_number, reached)
  local marker = M.result_marker(reached.proposal_id, reached.decision, reached.dedup_key)
  local state_marker = M.state_marker(reached.proposal_id, "ready", reached.dedup_key)
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  local verdict_summary = build_verdict_summary(reached.angle_results)
  local body = M.comment_string("decision_prefix") .. tostring(reached.decision)
  if verdict_summary ~= nil then
    body = body .. "\n" .. verdict_summary
  end
  body = body
    .. "\n\n" .. body_text
    .. "\n\n" .. state_marker
    .. "\n" .. marker
    .. "\n" .. ai_sentinel
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = body,
    -- Include the consensus M._dedup_key (version) so a new decision/version writes a fresh result
    -- marker instead of being suppressed by an older same-direction github-proxy comment marker.
    dedup_key = tostring(reached.proposal_id) .. "/comment/" .. tostring(reached.decision)
      .. "/" .. (tostring(reached.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(reached.source_ref),
  }
end

function M.build_converge_round_comment_request(repo, issue_number, unresolved, round, marker_body)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = build_convergence_display(M.comment_string("convergence_round_prefix"), unresolved, round)
      .. "\n\n" .. tostring(marker_body)
      .. "\n" .. ai_sentinel,
    dedup_key = M._dedup_key({
      "converge-round",
      "comment",
      tostring(unresolved.proposal_id),
      tostring(round),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.build_review_converge_round_comment_request(repo, issue_number, unresolved, issue_proposal_id, round, marker_body, source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = unresolved.pr_number or select(2, M.parse_pr_source_ref(unresolved.source_ref)),
  }, build_convergence_display(M.comment_string("pr_review_convergence_round_prefix"), unresolved, round)
    .. "\n\n" .. tostring(marker_body)
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-converge-round",
    "comment",
    tostring(issue_proposal_id),
    tostring(round),
    tostring(unresolved.dedup_key),
  }), source_ref or unresolved.source_ref)
end

function M.build_issue_review_converge_round_comment_request(repo, issue_number, unresolved, issue_proposal_id, round, marker_body, source_ref)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = build_convergence_display(M.comment_string("pr_review_convergence_round_prefix"), unresolved, round)
      .. "\n\n" .. tostring(marker_body)
      .. "\n" .. ai_sentinel,
    dedup_key = M._dedup_key({
      "review-converge-round",
      "comment",
      tostring(issue_proposal_id),
      tostring(round),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_reconcile_label_request(repo, issue_number, reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "reconcile",
      "label",
      tostring(reconcile.dedup_key),
    }),
    reconcile.source_ref
  )
end

function M.build_review_reconcile_label_request(repo, issue_number, review_reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "review-reconcile",
      "label",
      tostring(review_reconcile.dedup_key),
    }),
    review_reconcile.source_ref
  )
end

function M.build_fix_reconcile_label_request(repo, issue_number, fix_reconcile)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "fix-reconcile",
      "label",
      tostring(fix_reconcile.dedup_key),
    }),
    fix_reconcile.source_ref
  )
end

function M.build_dependency_hold_comment_request(repo, issue_number, proposal_id, version, gate, marker, source_ref)
  local reason = M.neutralize_untrusted_comment_text(gate and gate.reason or "")
  if reason == "" then
    reason = gate and gate.kind or "dependency-hold"
  end
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("dependency_hold_prefix") .. tostring(gate and gate.kind or "unknown")
      .. "\n\n" .. M.comment_string("reason_inline_label") .. reason
      .. "\n\n" .. tostring(marker),
    dedup_key = M._dedup_key({ "dependency", "comment", tostring(proposal_id), tostring(version), tostring(gate and gate.kind or "unknown") }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
  local version = M.reconcile_state_version(reconcile.base_version, reconcile.round)
  local marker = M.reconcile_marker(reconcile.proposal_id, reconcile.base_version, reconcile.round, action)
  local state_marker = M.state_marker(reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("reconcile_action_prefix") .. tostring(action)
      .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
      .. "\n\n"
      .. state_marker .. "\n" .. marker
      .. "\n" .. ai_sentinel,
    dedup_key = M._dedup_key({
      "reconcile",
      "comment",
      tostring(reconcile.dedup_key),
    }),
    source_ref = M.normalize_source_ref(reconcile.source_ref),
  }
end

function M.build_fix_reconcile_comment_request(repo, issue_number, fix_reconcile, action, reason)
  local version = M.fix_reconcile_state_version(fix_reconcile.issue_version)
  local marker = M.fix_reconcile_marker(fix_reconcile.proposal_id, fix_reconcile.issue_version, action)
  local state_marker = M.state_marker(fix_reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = M.parse_pr_source_ref(fix_reconcile.source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("fix_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "fix-reconcile",
    "comment",
    tostring(fix_reconcile.dedup_key),
  }), fix_reconcile.source_ref)
end

function M.build_review_reconcile_comment_request(repo, issue_number, review_reconcile, action, reason)
  local version = M.review_reconcile_state_version(review_reconcile.issue_version, review_reconcile.round)
  local marker = M.review_reconcile_marker(review_reconcile.proposal_id, review_reconcile.issue_version, review_reconcile.round, action)
  local state_marker = M.state_marker(review_reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local _, pr_number = M.parse_pr_source_ref(review_reconcile.source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("review_reconcile_action_prefix") .. tostring(action)
    .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
    .. "\n\n"
    .. state_marker .. "\n" .. marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-reconcile",
    "comment",
    tostring(review_reconcile.dedup_key),
  }), review_reconcile.source_ref)
end

function M.build_intake_decision_comment_request(repo, issue_number, candidate, decision, reason)
  local marker = M.intake_decision_marker(candidate.proposal_id, decision, candidate.dedup_key)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  if safe_reason == "" then
    safe_reason = M.comment_string("no_reason_provided")
  end
  if #safe_reason > M._max_meta_reason_len then
    safe_reason = M.truncate_utf8(safe_reason, M._max_meta_reason_len)
  end
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("intake_decision_prefix") .. tostring(decision)
      .. "\n\n" .. M.comment_string("reason_block_label") .. "\n" .. safe_reason
      .. "\n\n" .. marker,
    dedup_key = M._dedup_key({
      "intake",
      "comment",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    source_ref = M.normalize_source_ref(candidate.source_ref),
  }
end

function M.build_intake_enabled_label_request(repo, issue_number, candidate)
  return M.build_label_request(
    repo,
    issue_number,
    { M._enabled_label },
    {},
    M._dedup_key({
      "intake",
      "label",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    candidate.source_ref
  )
end

function M.build_implementing_label_request(repo, issue_number, ready)
  return M.build_state_label_request(
    repo,
    issue_number,
    "implementing",
    M._dedup_key({
      "implement",
      "label",
      "implementing",
      tostring(ready.dedup_key),
    }),
    ready.source_ref
  )
end

function M.build_impl_failed_label_request(repo, issue_number, ready, reason)
  return M.build_state_label_request(
    repo,
    issue_number,
    "impl-failed",
    M._dedup_key({
      "implement",
      "label",
      "impl-failed",
      tostring(reason or "failed"),
      tostring(ready.dedup_key),
    }),
    ready.source_ref
  )
end

function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha, base_branch, base_sha)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid implementing branch")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid implementing head_sha")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid implementing base_branch")
  end
  if not M._is_git_sha(base_sha) then
    error("github-devloop: invalid implementing base_sha")
  end
  local marker = M.implementing_marker(ready.proposal_id, ready.dedup_key, branch, head_sha, base_branch, base_sha)
  local state_marker = M.state_marker(ready.proposal_id, "implementing", ready.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("implementation_started")
      .. "\n\n" .. M.comment_string("worktree_label") .. tostring(worktree)
      .. "\n" .. M.comment_string("branch_label") .. tostring(branch)
      .. "\n" .. M.comment_string("head_label") .. tostring(head_sha)
      .. "\n" .. M.comment_string("base_branch_label") .. tostring(base_branch)
      .. "\n" .. M.comment_string("base_head_label") .. tostring(base_sha)
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = M._dedup_key({
      "implement",
      "comment",
      "implementing",
      tostring(ready.dedup_key),
    }),
    source_ref = M.normalize_source_ref(ready.source_ref),
  }
end

function M.build_impl_failure_comment_request(repo, issue_number, ready, reason, detail)
  local safe_reason = M.sanitize_key(reason or "failed"):gsub("/", "-")
  local text = tostring(detail or "")
  if #text > M._max_impl_output_len then
    text = M.truncate_utf8(text, M._max_impl_output_len)
  end
  if text == "" then
    text = M.comment_string("no_implementation_output")
  end
  text = M.neutralize_untrusted_comment_text(text)

  local marker = M.impl_failure_marker(ready.proposal_id, ready.dedup_key, safe_reason)
  local state_marker = M.state_marker(ready.proposal_id, "impl-failed", ready.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("implementation_failed_prefix") .. safe_reason
      .. "\n\n" .. text
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = M._dedup_key({
      "implement",
      "comment",
      "failure",
      safe_reason,
      tostring(ready.dedup_key),
    }),
    source_ref = M.normalize_source_ref(ready.source_ref),
  }
end

function M.build_pr_open_request(repo, issue_number, proposal_id, current, title, branch, head_sha, base_branch)
  if type(current) ~= "table" or current.state ~= "implementing" or not M._is_bounded_string(current.version, M._max_dedup_len) then
    error("github-devloop: invalid implementing state for pr request")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid pr branch")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid pr head_sha")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid pr base_branch")
  end
  local bounded_title = tostring(title or "")
  if bounded_title == "" then
    bounded_title = "github-devloop implementation for #" .. tostring(issue_number)
  end
  if #bounded_title > M._max_pr_title_len then
    bounded_title = M.truncate_utf8(bounded_title, M._max_pr_title_len)
  end
  local body = "github-devloop implementation PR for issue #" .. tostring(issue_number)
    .. "\n\n" .. M.pr_origin_marker(proposal_id, issue_number, branch, current.version, base_branch)
  local add_labels, remove_labels = M.state_label_changes("pr-open")
  return {
    schema = "github-proxy.pr-open.v1",
    repo = repo,
    issue_number = issue_number,
    proposal_id = proposal_id,
    impl_version = current.version,
    expected_state = current.state,
    expected_version = current.version,
    branch = branch,
    head_sha = head_sha,
    base_branch = base_branch,
    title = bounded_title,
    body = body,
    issue_comment_body_template = M.comment_string("pr_opened_prefix") .. "{{pr_number}}"
      .. "\n\n" .. M.state_marker(proposal_id, "pr-open", current.version)
      .. "\n" .. M.pr_link_marker_template(proposal_id, branch, current.version, base_branch),
    issue_label_add = add_labels,
    issue_label_remove = remove_labels,
    dedup_key = M._dedup_key({
      "open-pr",
      tostring(proposal_id),
      tostring(current.version),
      tostring(branch),
    }),
    source_ref = {
      kind = "external",
      ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
    },
  }
end

function M.build_pr_open_comment_request(repo, issue_number, proposal_id, current, pr_number, branch, base_branch, source_ref)
  local state_marker = M.state_marker(proposal_id, "pr-open", current.version)
  local link_marker = M.pr_link_marker(proposal_id, pr_number, branch, current.version, base_branch)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = M.comment_string("pr_opened_prefix") .. tostring(pr_number)
      .. "\n\n" .. state_marker
      .. "\n" .. link_marker,
    dedup_key = M._dedup_key({
      "open-pr",
      "comment",
      tostring(proposal_id),
      tostring(current.version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_pr_open_label_request(repo, issue_number, proposal_id, current, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "pr-open",
    M._dedup_key({
      "open-pr",
      "label",
      tostring(proposal_id),
      tostring(current.version),
    }),
    source_ref
  )
end

function M.build_reviewing_comment_request(repo, issue_number, origin, pr_number, source_ref)
  local state_marker = M.state_marker(origin.proposal_id, "reviewing", origin.impl_version)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("pr_ready_for_review")
    .. "\n\n" .. state_marker, M._dedup_key({
    "observe-pr",
    "comment",
    tostring(origin.proposal_id),
    tostring(origin.impl_version),
    tostring(pr_number),
  }), source_ref)
end

function M.build_reviewing_label_request(repo, issue_number, origin, pr_number, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "observe-pr",
      "label",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref
  )
end

function M.build_review_result_label_request(repo, issue_number, issue_proposal_id, reached, source_ref)
  local to_state = reached.reflection_checkpoint and "review-meta"
    or reached.decision == "approve" and "merge-ready"
    or "fixing"
  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    M._dedup_key({
      "review-result",
      "label",
      tostring(issue_proposal_id),
      tostring(reached.decision),
      tostring(reached.dedup_key),
    }),
    source_ref
  )
end

function M.build_review_result_comment_request(repo, issue_number, issue_proposal_id, issue_version, reached, source_ref)
  local to_state = reached.reflection_checkpoint and "review-meta"
    or reached.decision == "approve" and "merge-ready"
    or "fixing"
  local state_marker = M.state_marker(issue_proposal_id, to_state, issue_version)
  local fix_round = nil
  if reached.decision == "reject" then
    fix_round = M.version_fix_round(issue_version)
  end
  local blocking_gap = bounded_blocking_gap(M, reached)
  local marker = M.review_result_marker(reached.proposal_id, issue_proposal_id, reached.decision, reached.dedup_key, fix_round, blocking_gap)
  local reflection_marker = ""
  if reached.reflection_checkpoint then
    reflection_marker = "\n" .. M.fix_reflection_marker(issue_proposal_id, reached.dedup_key, "checkpoint", issue_version, fix_round)
  end
  local merge_marker = ""
  if reached.decision == "approve" then
    local _, pr_number, _, reviewed_head_sha = M.parse_pr_review_proposal_id(reached.proposal_id)
    merge_marker = "\n" .. M.merge_ready_marker(issue_proposal_id, pr_number, issue_version, reached.proposal_id, reached.dedup_key, reviewed_head_sha)
  end
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  local verdict_summary = build_verdict_summary(reached.angle_results)
  local body = M.comment_string("pr_review_decision_prefix") .. tostring(reached.decision)
  if verdict_summary ~= nil then
    body = body .. "\n" .. verdict_summary
  end
  if reached.decision == "reject" and blocking_gap ~= nil then
    body = body .. "\n" .. M.comment_string("blocking_gap_label") .. M.neutralize_untrusted_comment_text(blocking_gap)
  end
  local _, pr_number = M.parse_pr_source_ref(source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, body
    .. "\n\n" .. body_text
    .. "\n\n" .. state_marker
    .. "\n" .. marker
    .. reflection_marker
    .. merge_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-result",
    "comment",
    tostring(issue_proposal_id),
    tostring(reached.decision),
    tostring(reached.dedup_key),
  }), source_ref)
end

function M.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, gate_baseline_sha, source_ref)
  local safe_reason = M.sanitize_key(reason or "gate-failed", false):gsub("/", "-")
  local display_reason = M.neutralize_untrusted_comment_text(reason or "gate-failed")
  if display_reason == "" then
    display_reason = "gate-failed"
  end
  if gate_baseline_sha ~= nil and not M._is_git_sha(gate_baseline_sha) then
    error("github-devloop: invalid merge-gate baseline sha")
  end
  local test_command = M.neutralize_untrusted_comment_text(M.test_command())
  local state_marker = M.state_marker(merge_ready.proposal_id, "fixing", fix_version)
  local marker = M.merge_gate_marker(
    merge_ready.proposal_id,
    merge_ready.pr_number,
    fix_version,
    merge_ready.review_proposal_id,
    merge_ready.review_dedup_key,
    merge_ready.reviewed_head_sha,
    gate_baseline_sha,
    safe_reason
  )
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, M.comment_string("merge_gate_failed_prefix") .. display_reason
    .. "\n" .. M.comment_string("reproduce_locally_prefix") .. test_command .. M.comment_string("reproduce_locally_suffix")
    .. "\n\n" .. state_marker
    .. "\n" .. marker, M._dedup_key({
    "merge",
    "comment",
    "fixing",
    tostring(merge_ready.proposal_id),
    tostring(merge_ready.version),
    safe_reason,
  }), source_ref)
end

function M.build_fix_reviewing_label_request(repo, issue_number, fix, new_head_sha, new_version)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "fix",
      "label",
      tostring(fix.proposal_id),
      tostring(fix.review_dedup_key),
      tostring(new_head_sha),
    }),
    fix.source_ref
  )
end

function M.build_fix_reviewing_comment_request(repo, issue_number, fix, old_head_sha, new_head_sha, new_version)
  local state_marker = M.state_marker(fix.proposal_id, "reviewing", new_version or fix.version)
  local marker = M.fix_marker(fix.proposal_id, fix.review_proposal_id, fix.review_dedup_key, old_head_sha, new_head_sha)
  local summary = ""
  if fix.fix_summary ~= nil and tostring(fix.fix_summary) ~= "" then
    summary = "\n" .. M.comment_string("fix_round_summary_label") .. M.neutralize_untrusted_comment_text(fix.fix_summary)
  end
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, M.comment_string("fix_pushed_for_rereview")
    .. "\n\n" .. M.comment_string("previous_reviewed_head_label") .. tostring(old_head_sha)
    .. "\n" .. M.comment_string("new_head_label") .. tostring(new_head_sha)
    .. summary
    .. "\n\n" .. state_marker
    .. "\n" .. marker, M._dedup_key({
    "fix",
    "comment",
    tostring(fix.proposal_id),
    tostring(fix.review_dedup_key),
    tostring(new_head_sha),
  }), fix.source_ref)
end

function M.raise_fix_reviewing(opts)
  opts = opts or {}
  local dept = tostring(opts.dept or "unknown")
  local repo = opts.repo
  local issue_number = opts.issue_number
  local fix = opts.fix or {}
  local old_head_sha = opts.old_head_sha
  local new_head_sha = opts.new_head_sha
  local new_version = opts.new_version or M.next_fix_version(fix.version)
  local reason = opts.reason
  local current_state = opts.current_state or { state = "fixing", version = fix.version }
  if opts.fix_summary ~= nil or opts.clear_fix_summary == true then
    fix.fix_summary = opts.fix_summary
  end

  M.log_cas_decision(dept, fix.proposal_id, current_state, "fixing", "reviewing", "applied", reason)
  local comment_request = M.build_fix_reviewing_comment_request(repo, issue_number, fix, old_head_sha, new_head_sha, new_version)
  local label_request = M.build_fix_reviewing_label_request(repo, issue_number, fix, new_head_sha, new_version)
  local add_labels, remove_labels = M.state_label_changes("reviewing")
  local reviewing_payload = M.build_devloop_reviewing_payload({
    proposal_id = fix.proposal_id,
    impl_version = new_version,
  }, fix.pr_number, fix.source_ref)
  M.log_apply(dept, fix.proposal_id, "reviewing", new_version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_reviewing",
  })
  M.log_raise(dept, fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if issue_number ~= nil then
    M.log_raise(dept, fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  M.log_raise(dept, fix.proposal_id, "devloop_reviewing", reviewing_payload)
end

function M.build_merge_head_reviewing_label_request(repo, issue_number, merge_ready, new_head_sha, new_version, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "merge",
      "label",
      "reviewing",
      tostring(merge_ready.proposal_id),
      tostring(new_version),
      tostring(new_head_sha),
    }),
    source_ref
  )
end

function M.build_merge_head_reviewing_comment_request(repo, issue_number, merge_ready, old_head_sha, new_head_sha, new_version, source_ref)
  local state_marker = M.state_marker(merge_ready.proposal_id, "reviewing", new_version)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, M.comment_string("pr_head_advanced")
    .. "\n\n" .. M.comment_string("previous_reviewed_head_label") .. tostring(old_head_sha)
    .. "\n" .. M.comment_string("current_head_label") .. tostring(new_head_sha)
    .. "\n\n" .. state_marker, M._dedup_key({
    "merge",
    "comment",
    "reviewing",
    tostring(merge_ready.proposal_id),
    tostring(new_version),
    tostring(new_head_sha),
  }), source_ref)
end

function M.build_merging_comment_body(merge_ready)
  return M.comment_string("is_merging_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
end

function M.build_merged_comment_body(merge_ready)
  return M.comment_string("merged_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
    .. "\n" .. M.state_marker(merge_ready.proposal_id, "merged", merge_ready.version)
    .. "\n" .. M.merged_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
end

end

return S
