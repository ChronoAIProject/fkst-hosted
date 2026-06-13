local S = {}

function S.install(M)
local function replay_pr_open(dept, issue, state, row, facts, tools)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil or M.strip_transition_version_suffixes(state.version) ~= M.strip_transition_version_suffixes(link.impl_version) then
    return tools.log_skip(dept, proposal_id, state, "pr-open", "reviewing", "skip-foreign(pr-link)", "pr-open replay requires a same-version pr-link marker")
  end
  for _, item in ipairs(facts.snapshot.prs or {}) do
    if tostring(item.number or "") == tostring(link.pr_number or "") then
      local pr = item.current or {}
      if tostring(pr.state or ""):lower() ~= "open" then
        local ready_version = M.orphaned_pr_ready_version(state)
        local ready_payload = M.build_devloop_ready_payload({
          proposal_id = proposal_id,
          dedup_key = ready_version,
          source_ref = issue.source_ref,
        })
        M.log_cas_decision(dept, proposal_id, state, "pr-open", "ready", "applied(orphaned-pr-closed)", "linked PR is closed; re-driving implementation to replace it")
        return tools.raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, {
          { queue = "devloop_ready", payload = ready_payload },
        })
      end
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
      local reviewing_payload = M.build_devloop_reviewing_payload({
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref, fields.version)
      local reviewing_comment = M.build_reviewing_comment_request(issue.repo, issue.number, {
        proposal_id = fields.proposal_id,
        impl_version = fields.version,
      }, fields.pr_number, fields.source_ref)
      M.log_cas_decision(dept, proposal_id, state, "pr-open", "reviewing", "applied(replay)", "linked PR head/base match pr-link marker")
      return tools.raise_effects(dept, proposal_id, "pr-open", state.version, { add = {}, remove = {} }, {
        { queue = "github-proxy.github_pr_comment_request", payload = reviewing_comment },
        { queue = "devloop_reviewing", payload = reviewing_payload },
      })
    end
  end
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
    return tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return tools.log_skip(dept, proposal_id, state, "reviewing", "reviewing", "skip-stale(pr-closed)", "linked PR is not open")
  end
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
  local payload = M.build_devloop_reviewing_payload({
    proposal_id = fields.proposal_id,
    impl_version = fields.version,
  }, fields.pr_number, fields.source_ref, fields.version)
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
  end
  table.insert(effects, { queue = "devloop_reviewing", payload = payload })
  return tools.raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, effects)
end

function M.install_pr_review_replayers(replayers, tools)
  replayers["pr-open"] = function(dept, issue, state, row, facts)
    return replay_pr_open(dept, issue, state, row, facts, tools)
  end
  replayers.reviewing = function(dept, issue, state, row, facts)
    return replay_reviewing(dept, issue, state, row, facts, tools)
  end
end

end

return S
