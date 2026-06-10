local S = {}

function S.install(M)
local ai_sentinel = "⟦AI:FKST⟧"
local convergence_suffix = " — no three-angle consensus; narrowing"
local display_separator = " — "
local narrowed_question_label = "Narrowed question: "
local angle_stances_label = "Angle stances:"
local verdict_summary_label = "Three-angle verdicts: "
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
    text = M._utf8_safe_truncate(text, cap)
  end
  text = M.neutralize_untrusted_comment_text(text)
  if #text > cap then
    text = M._utf8_safe_truncate(text, cap)
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
    header .. " " .. tostring(round) .. convergence_suffix,
  }
  local question = bounded_neutralized_text(unresolved and unresolved.narrowed_question or "", max_display_question_len)
  if question ~= "" then
    table.insert(lines, "")
    table.insert(lines, narrowed_question_label .. question)
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
    table.insert(lines, angle_stances_label)
    for _, line in ipairs(angle_lines) do
      table.insert(lines, line)
    end
  end
  local body = table.concat(lines, "\n")
  if #body > max_display_block_len then
    body = M._utf8_safe_truncate(body, max_display_block_len)
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
  local summary = verdict_summary_label .. table.concat(parts, " ")
  if #summary > max_verdict_summary_len then
    summary = M._utf8_safe_truncate(summary, max_verdict_summary_len)
  end
  return summary
end

local function build_comment_evidence_digest(M, comments)
  local text = table.concat(M.comment_bodies(comments), "\n\n")
  text = text:gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return "(review rounds are recorded on the parent PR comments)"
  end
  if #text > max_verdict_summary_len then
    text = M._utf8_safe_truncate(text, max_verdict_summary_len)
  end
  return text
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
    text = M._utf8_safe_truncate(text, M._max_blocking_gap_len)
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
    body = "github-devloop thinking: consensus started\n\n"
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
  local body = "github-devloop decision: " .. tostring(reached.decision)
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
    body = build_convergence_display("github-devloop convergence round", unresolved, round)
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
  }, build_convergence_display("github-devloop PR review convergence round", unresolved, round)
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
    body = build_convergence_display("github-devloop PR review convergence round", unresolved, round)
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

function M.build_reconcile_comment_request(repo, issue_number, reconcile, action, reason)
  local version = M.reconcile_state_version(reconcile.base_version, reconcile.round)
  local marker = M.reconcile_marker(reconcile.proposal_id, reconcile.base_version, reconcile.round, action)
  local state_marker = M.state_marker(reconcile.proposal_id, "blocked", version)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop reconcile action: " .. tostring(action)
      .. "\n\nReason:\n" .. safe_reason
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
  }, "github-devloop fix reconcile action: " .. tostring(action)
    .. "\n\nReason:\n" .. safe_reason
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
  }, "github-devloop review reconcile action: " .. tostring(action)
    .. "\n\nReason:\n" .. safe_reason
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
    safe_reason = "(no reason provided)"
  end
  if #safe_reason > M._max_meta_reason_len then
    safe_reason = M._utf8_safe_truncate(safe_reason, M._max_meta_reason_len)
  end
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop intake decision: " .. tostring(decision)
      .. "\n\nReason:\n" .. safe_reason
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
    body = "github-devloop implementation started"
      .. "\n\nWorktree: " .. tostring(worktree)
      .. "\nBranch: " .. tostring(branch)
      .. "\nHead: " .. tostring(head_sha)
      .. "\nBase branch: " .. tostring(base_branch)
      .. "\nBase head: " .. tostring(base_sha)
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
    text = M._utf8_safe_truncate(text, M._max_impl_output_len)
  end
  if text == "" then
    text = "(no implementation output)"
  end
  text = M.neutralize_untrusted_comment_text(text)

  local marker = M.impl_failure_marker(ready.proposal_id, ready.dedup_key, safe_reason)
  local state_marker = M.state_marker(ready.proposal_id, "impl-failed", ready.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop implementation failed: " .. safe_reason
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
    bounded_title = M._utf8_safe_truncate(bounded_title, M._max_pr_title_len)
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
    issue_comment_body_template = "github-devloop PR opened: #{{pr_number}}"
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
    body = "github-devloop PR opened: #" .. tostring(pr_number)
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
  }, "github-devloop PR is ready for review"
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
  local to_state = reached.decision == "approve" and "merge-ready" or "fixing"
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
  local to_state = reached.decision == "approve" and "merge-ready" or "fixing"
  local state_marker = M.state_marker(issue_proposal_id, to_state, issue_version)
  local fix_round = nil
  if reached.decision == "reject" then
    fix_round = M.version_fix_round(issue_version)
  end
  local blocking_gap = bounded_blocking_gap(M, reached)
  local marker = M.review_result_marker(reached.proposal_id, issue_proposal_id, reached.decision, reached.dedup_key, fix_round, blocking_gap)
  local merge_marker = ""
  if reached.decision == "approve" then
    local _, pr_number, _, reviewed_head_sha = M.parse_pr_review_proposal_id(reached.proposal_id)
    merge_marker = "\n" .. M.merge_ready_marker(issue_proposal_id, pr_number, issue_version, reached.proposal_id, reached.dedup_key, reviewed_head_sha)
  end
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  local verdict_summary = build_verdict_summary(reached.angle_results)
  local body = "github-devloop PR review decision: " .. tostring(reached.decision)
  if verdict_summary ~= nil then
    body = body .. "\n" .. verdict_summary
  end
  if reached.decision == "reject" and blocking_gap ~= nil then
    body = body .. "\nBlocking gap: " .. M.neutralize_untrusted_comment_text(blocking_gap)
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
    .. merge_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-result",
    "comment",
    tostring(issue_proposal_id),
    tostring(reached.decision),
    tostring(reached.dedup_key),
  }), source_ref)
end

function M.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, source_ref)
  local safe_reason = M.sanitize_key(reason or "gate-failed", false):gsub("/", "-")
  local display_reason = M.neutralize_untrusted_comment_text(reason or "gate-failed")
  if display_reason == "" then
    display_reason = "gate-failed"
  end
  local state_marker = M.state_marker(merge_ready.proposal_id, "fixing", fix_version)
  local marker = M.merge_gate_marker(
    merge_ready.proposal_id,
    merge_ready.pr_number,
    fix_version,
    merge_ready.review_proposal_id,
    merge_ready.review_dedup_key,
    merge_ready.reviewed_head_sha,
    safe_reason
  )
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, "github-devloop merge gate failed: " .. display_reason
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
    summary = "\nFix-round summary: " .. M.neutralize_untrusted_comment_text(fix.fix_summary)
  end
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, "github-devloop fix pushed for re-review"
    .. "\n\nPrevious reviewed head: " .. tostring(old_head_sha)
    .. "\nNew head: " .. tostring(new_head_sha)
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
  }, "github-devloop PR head advanced after merge approval; re-entering review"
    .. "\n\nPrevious reviewed head: " .. tostring(old_head_sha)
    .. "\nCurrent head: " .. tostring(new_head_sha)
    .. "\n\n" .. state_marker, M._dedup_key({
    "merge",
    "comment",
    "reviewing",
    tostring(merge_ready.proposal_id),
    tostring(new_version),
    tostring(new_head_sha),
  }), source_ref)
end

function M.build_fix_review_meta_label_request(repo, issue_number, fix, reason)
  return M.build_state_label_request(
    repo,
    issue_number,
    "review-meta",
    M._dedup_key({
      "fix",
      "label",
      "review-meta",
      tostring(reason or "no-fix"),
      tostring(fix.review_dedup_key),
    }),
    fix.source_ref
  )
end

function M.build_fix_review_meta_comment_request(repo, issue_number, fix, reason, detail)
  local safe_reason = M.sanitize_key(reason or "no-fix"):gsub("/", "-")
  local text = tostring(detail or "")
  if #text > M._max_impl_output_len then
    text = M._utf8_safe_truncate(text, M._max_impl_output_len)
  end
  if text == "" then
    text = "(no fix output)"
  end
  text = M.neutralize_untrusted_comment_text(text)
  local state_marker = M.state_marker(fix.proposal_id, "review-meta", fix.version)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, "github-devloop fix escalated to review-meta: " .. safe_reason
    .. "\n\n" .. text
    .. "\n\n" .. state_marker, M._dedup_key({
    "fix",
    "comment",
    "review-meta",
    safe_reason,
    tostring(fix.dedup_key),
  }), fix.source_ref)
end

function M.build_review_meta_label_request(repo, issue_number, review_meta, action, version)
  local to_state = action == "fix" and "fixing" or "blocked"
  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    M._dedup_key({
      "review-meta",
      "label",
      tostring(action),
      tostring(review_meta.dedup_key),
      tostring(version or review_meta.version),
    }),
    review_meta.source_ref
  )
end

function M.build_review_meta_comment_request(repo, issue_number, review_meta, action, reason, version, blocking_gap)
  local to_state = action == "fix" and "fixing" or "blocked"
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local state_version = version or review_meta.version
  local action_text = tostring(action)
  if action == "spec-amendment" then
    action_text = "blocked-pending-spec"
  end
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = review_meta.pr_number,
  }, "github-devloop review-meta action: " .. action_text
    .. "\n\nReason:\n" .. safe_reason
    .. "\n\n" .. M.state_marker(review_meta.proposal_id, to_state, state_version)
    .. "\n" .. M.review_meta_marker(review_meta.proposal_id, review_meta.dedup_key, action, state_version, blocking_gap, reason), M._dedup_key({
    "review-meta",
    "comment",
    tostring(review_meta.dedup_key),
    tostring(state_version),
  }), review_meta.source_ref)
end

function M.build_spec_amendment_issue_create_request(repo, issue_number, review_meta, title_brief, reason, comments)
  local title = "Spec amendment needed: " .. tostring(title_brief or ("Issue #" .. tostring(issue_number or "unknown")))
  if #title > M._max_title_len then
    title = M._utf8_safe_truncate(title, M._max_title_len)
  end
  local evidence = build_comment_evidence_digest(M, comments)
  local body = "Spec flaw statement:\n" .. M.neutralize_untrusted_comment_text(reason or "")
    .. "\n\nEvidence digest:\n" .. M.neutralize_untrusted_comment_text(evidence)
    .. "\n\nParent issue: #" .. tostring(issue_number or "unknown")
    .. "\nParent PR: #" .. tostring(review_meta.pr_number)
    .. "\nReview proposal: " .. tostring(review_meta.review_proposal_id)
    .. "\nReview dedup: " .. tostring(review_meta.dedup_key)
    .. "\n\nThis issue requests a spec revision only. Do not edit the human-authored parent issue text."
  if #body > M._max_body_len then
    body = M._utf8_safe_truncate(body, M._max_body_len)
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = title,
    body = body,
    labels = json.decode("[]"),
    dedup_key = M._dedup_key({
      "spec-amendment",
      tostring(review_meta.proposal_id),
      tostring(review_meta.dedup_key),
    }),
    parent_comment_target = {
      repo = repo,
      pr_number = review_meta.pr_number,
    },
    source_ref = M.normalize_source_ref(review_meta.source_ref),
  }
end
end

return S
