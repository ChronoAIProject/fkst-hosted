local S = {}

function S.install(M)
local function linked_pr_state(pr)
  return tostring(pr and pr.state or ""):upper()
end

local function merged_head_sha(pr)
  local head_sha = tostring(pr and pr.head_sha or "")
  return M._is_git_sha(head_sha) and head_sha or nil
end

local function mark_child_closed_unmerged(dept, issue, state, proposal_id, link, tools, outcome, reason)
  local version = M.strip_transition_version_suffixes(state and state.version)
  local pr_number = link and link.pr_number
  local source_ref = pr_number ~= nil and M.pr_source_ref(issue.repo, pr_number) or issue.source_ref
  local comment_request = M.build_entity_comment_request({
    kind = "pr",
    repo = issue.repo,
    number = pr_number,
  }, "github-devloop marked delegated PR child closed without merge"
    .. "\n\nReason: " .. tostring(reason or "closed without merge")
    .. "\n\n" .. M.state_marker(proposal_id, "closed-unmerged", version)
    .. "\n" .. "⟦AI:FKST⟧", M._dedup_key({
    "child-pr",
    "closed-unmerged",
    tostring(proposal_id),
    tostring(version),
    tostring(pr_number),
  }), source_ref)
  local label_request = M.build_reconcile_pr_state_label_request(
    issue.repo,
    issue.number,
    pr_number,
    proposal_id,
    "closed-unmerged",
    version,
    source_ref
  )
  M.log_cas_decision(dept, proposal_id, state, state and state.state or "pr-open", "closed-unmerged", outcome, reason)
  local add_labels, remove_labels = M.state_label_changes("closed-unmerged")
  return tools.raise_effects(dept, proposal_id, "closed-unmerged", version, { add = add_labels, remove = remove_labels }, {
    { queue = "github-proxy.github_pr_comment_request", payload = comment_request },
    { queue = "github-proxy.github_issue_label_request", payload = label_request },
  })
end

local function mark_issue_merged_from_linked_pr(dept, issue, state, proposal_id, link, pr, tools)
  local head_sha = merged_head_sha(pr)
  if head_sha == nil then
    return tools.log_skip(dept, proposal_id, state, state.state, "merged", "skip-foreign(head)", "merged linked PR head sha is missing")
  end
  local merged_body = M.comment_string("merged_pr_prefix") .. tostring(link.pr_number)
    .. "\n\n" .. M.state_marker(proposal_id, "merged", state.version)
    .. "\n" .. M.merged_marker(proposal_id, link.pr_number, state.version, head_sha)
  local source_ref = M.pr_source_ref(issue.repo, link.pr_number)
  local comment_request = M.build_entity_comment_request({
    kind = "issue",
    repo = issue.repo,
    number = issue.number,
  }, merged_body, M._dedup_key({
    "orphaned-pr",
    "merged",
    tostring(proposal_id),
    tostring(state.version),
    tostring(link.pr_number),
    tostring(head_sha),
  }), issue.source_ref)
  local label_request = M.build_state_label_request(
    issue.repo,
    issue.number,
    "merged",
    M._dedup_key({
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

local function terminal_linked_pr_action(dept, issue, state, proposal_id, link, pr, facts, tools)
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
  if link == nil or M.strip_transition_version_suffixes(state.version) ~= M.strip_transition_version_suffixes(link.impl_version) then
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
      if not M._is_git_sha(pr.head_sha) then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "linked PR head sha is missing")
      end
      local review_version = M.review_redrive_version(state, {
        repo = issue.repo,
        number = link.pr_number,
        head_sha = pr.head_sha,
      })
      local review_proposal_id = M.pr_review_proposal_id(issue.repo, link.pr_number, review_version, pr.head_sha)
      if M.has_any_review_result_marker(facts.snapshot.comments, review_proposal_id, proposal_id) then
        return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-idempotent(review result visible)", "review already produced a result")
      end
      local fields = tools.resolve_payload_fields(row, state, {
        issue = issue,
        state = state,
        link = link,
        proposal_id = proposal_id,
      })
      fields.version = review_version
      local reviewing_comment = M.build_reviewing_comment_request(issue.repo, issue.number, {
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
  local link = facts.link
  if link == nil then
    return tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-foreign(pr-link)", "reviewing recovery requires a pr-link marker")
  end
  local current_pr = tools.find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    local absent_redrive = redrive_absent_replacement_pr(dept, issue, state, proposal_id, link, facts, tools)
    if absent_redrive ~= nil then return absent_redrive end
    return tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local terminal = terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts, tools)
  if terminal ~= nil then return terminal end
  if not M._is_git_sha(current_pr.head_sha) then
    return tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-foreign(head)", "linked PR head sha is missing")
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
  local review_proposal_id = M.pr_review_proposal_id(issue.repo, fields.pr_number, fields.version, current_pr.head_sha)
  if M.has_any_review_result_marker(current_pr.comments, review_proposal_id, proposal_id) then
    tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-idempotent(review result visible)", "review already produced a result")
    return true
  end
  M.log_cas_decision(dept, proposal_id, state, "reviewing", "reviewing", "applied(replay)", "current PR head has no trusted review result")
  local effects = {}
  if tostring(fields.version or "") ~= tostring(state.version or "") then
    table.insert(effects, {
      queue = "github-proxy.github_pr_comment_request",
      payload = M.build_reviewing_comment_request(issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref),
    })
  elseif dept == "observe_pr" then
    table.insert(effects, {
      queue = "github-proxy.github_pr_comment_request",
      payload = M.build_reviewing_comment_request(issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref),
    })
  else
    table.insert(effects, {
      queue = "devloop_reviewing",
      payload = M.build_devloop_reviewing_payload({
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref, fields.version),
    })
  end
  return tools.raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, effects)
end

function M.install_pr_review_replayers(replayers, tools)
  replayers["pr-open"] = function(dept, issue, state, row, facts)
    return replay_pr_open(dept, issue, state, row, facts, tools)
  end
  replayers.reviewing = function(dept, issue, state, row, facts)
    return replay_reviewing(dept, issue, state, row, facts, tools)
  end
  tools.terminal_linked_pr_action = function(dept, issue, state, proposal_id, link, pr, facts)
    return terminal_linked_pr_action(dept, issue, state, proposal_id, link, pr, facts, tools)
  end
  M.terminal_linked_pr_action = tools.terminal_linked_pr_action
end

end

return S
