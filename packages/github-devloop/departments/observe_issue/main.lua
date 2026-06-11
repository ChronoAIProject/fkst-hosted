local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "devloop_ready",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_merge_ready",
  },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

local function raise_pr_open_reviewing(issue, proposal_id, state, link, snapshot)
  if link == nil or snapshot == nil then
    return false
  end
  for _, item in ipairs(snapshot.prs or {}) do
    if tostring(item.number or "") == tostring(link.pr_number or "") then
      local pr = item.current or {}
      if tostring(pr.state or ""):lower() ~= "open" then
        core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-stale(pr-closed)", "linked PR is not open")
        return false
      end
      if tostring(pr.head_ref_name or "") ~= tostring(link.branch or "") then
        core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "linked PR head branch does not match pr-link marker")
        return false
      end
      if tostring(pr.base_ref_name or "") ~= tostring(link.base_branch or "") then
        core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-foreign(base)", "linked PR base branch does not match pr-link marker")
        return false
      end
      if not core._is_git_sha(pr.head_sha) then
        core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-foreign(head)", "linked PR head sha is missing")
        return false
      end
      local review_proposal_id = core.pr_review_proposal_id(issue.repo, link.pr_number, state.version, pr.head_sha)
      if core.has_any_review_result_marker(snapshot.comments, review_proposal_id, proposal_id) then
        core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-idempotent(review result visible)", "review already produced a result")
        return false
      end
      local reviewing_payload = core.build_devloop_reviewing_payload({
        proposal_id = proposal_id,
        impl_version = state.version,
      }, link.pr_number, core.pr_source_ref(issue.repo, link.pr_number), state.version)
      local reviewing_comment = core.build_reviewing_comment_request(issue.repo, issue.number, {
        proposal_id = proposal_id,
        impl_version = state.version,
      }, link.pr_number, core.pr_source_ref(issue.repo, link.pr_number))
      core.log_apply("observe_issue", proposal_id, "pr-open", state.version, { add = {}, remove = {} }, {
        "github-proxy.github_pr_comment_request",
        "devloop_reviewing",
      })
      core.log_raise("observe_issue", proposal_id, "github-proxy.github_pr_comment_request", reviewing_comment)
      core.log_raise("observe_issue", proposal_id, "devloop_reviewing", reviewing_payload)
      return true
    end
  end
  core.log_cas_decision("observe_issue", proposal_id, state, "pr-open", "reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
  return false
end

local function raise_thinking_replay(issue, proposal_id, state, current, event_ts)
  local state_base_version = core.version_loop_round(state.version) > 0 and core.converge_base_version(state.version) or nil
  local latest = core.latest_complete_converge_round(current.comments, proposal_id, state_base_version, issue.source_ref)
  if latest ~= nil then
    local base_version = latest.version
    local next_n = latest.round + 1
    local next_dedup = base_version .. "/loop/" .. tostring(next_n)
    local content_fetch = core.context_fetch_ref_from_bundle({
      dept = "observe_issue",
      repo = issue.repo,
      issue_number = issue.number,
      proposal_id = proposal_id,
      version = next_dedup,
      tick = event_ts,
    })
    local proposal = core.build_board_loop_proposal(issue.repo, issue.number, {
      title = issue.title,
      updated_at = issue.updated_at,
    }, issue.source_ref, next_n, {
      narrowed_question = latest.narrowed_question,
      angle_digests = latest.angle_digests,
    }, event_ts, content_fetch)
    proposal.dedup_key = next_dedup
    if core.validate_proposal(proposal) then
      core.log_apply("observe_issue", proposal_id, "thinking", proposal.dedup_key, { add = {}, remove = {} }, {
        "consensus.proposal",
      })
      core.log_raise("observe_issue", proposal_id, "consensus.proposal", proposal)
    else
      log.warn("github-devloop dept=observe_issue proposal_id=" .. tostring(proposal_id) .. " tag=SKIP reason=cannot-rebuild-thinking-loop-proposal")
    end
    return
  end

  if core.version_loop_round(state.version) == 0 then
    issue.content_fetch = core.context_fetch_ref_from_bundle({
      dept = "observe_issue",
      repo = issue.repo,
      issue_number = issue.number,
      proposal_id = proposal_id,
      version = state.version,
      tick = event_ts,
    })
    local proposal = core.build_board_proposal(issue, event_ts)
    proposal.dedup_key = state.version
    if core.validate_proposal(proposal) then
      core.log_apply("observe_issue", proposal_id, "thinking", proposal.dedup_key, { add = {}, remove = {} }, {
        "consensus.proposal",
      })
      core.log_raise("observe_issue", proposal_id, "consensus.proposal", proposal)
    else
      log.warn("github-devloop dept=observe_issue proposal_id=" .. tostring(proposal_id) .. " tag=SKIP reason=cannot-rebuild-thinking-proposal")
    end
  else
    log.warn("github-devloop dept=observe_issue proposal_id=" .. tostring(proposal_id) .. " tag=SKIP reason=cannot-rebuild-incomplete-thinking-loop-marker")
  end
end

local function find_linked_pr(snapshot, pr_number)
  for _, item in ipairs(snapshot.prs or {}) do
    if tostring(item.number or "") == tostring(pr_number or "") then
      return item.current
    end
  end
  return nil
end

local function raise_fixing_replay(issue, proposal_id, state, link, snapshot)
  if link == nil or not core.fixing_version_matches_link(state.version, link.impl_version) then
    core.log_cas_decision("observe_issue", proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(pr-link)", "fixing recovery requires a same-version pr-link marker")
    return
  end
  local current_pr = find_linked_pr(snapshot, link.pr_number)
  if current_pr == nil then
    core.log_cas_decision("observe_issue", proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("observe_issue", proposal_id, state, "fixing", "fixing|reviewing", "skip-stale(pr-closed)", "linked PR is not open")
    return
  end
  if not core._is_git_sha(current_pr.head_sha) then
    core.log_cas_decision("observe_issue", proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(head)", "linked PR head sha is missing")
    return
  end

  local feedback = core.fixing_replay_feedback_fact(snapshot.comments, proposal_id, state.version)
  if feedback ~= nil then
    local fix_payload = core.build_devloop_fixing_payload({
      proposal_id = proposal_id,
      impl_version = state.version,
    }, link.pr_number, {
      review_proposal_id = feedback.review_proposal_id,
      review_dedup_key = feedback.review_dedup_key,
      reviewed_head_sha = feedback.reviewed_head_sha,
      blocking_gap = feedback.blocking_gap,
    }, core.pr_source_ref(issue.repo, link.pr_number))
    core.log_apply("observe_issue", proposal_id, "fixing", state.version, { add = {}, remove = {} }, {
      "devloop_fixing",
    })
    core.log_raise("observe_issue", proposal_id, "devloop_fixing", fix_payload)
    return
  end

  local new_version = core.next_fix_version(state.version)
  local reviewing_payload = core.build_devloop_reviewing_payload({
    proposal_id = proposal_id,
    impl_version = new_version,
  }, link.pr_number, core.pr_source_ref(issue.repo, link.pr_number), new_version)
  local comment_request = core.build_merge_head_reviewing_comment_request(
    issue.repo,
    issue.number,
    {
      proposal_id = proposal_id,
      pr_number = link.pr_number,
    },
    current_pr.head_sha,
    current_pr.head_sha,
    new_version,
    core.pr_source_ref(issue.repo, link.pr_number)
  )
  local label_request = core.build_state_label_request(issue.repo, issue.number, "reviewing", core._dedup_key({
    "observe",
    "fixing",
    "renormalize",
    tostring(proposal_id),
    tostring(new_version),
    tostring(link.pr_number),
  }), issue.source_ref)
  core.log_apply("observe_issue", proposal_id, "reviewing", new_version, { add = { "fkst-dev:reviewing" }, remove = { "fkst-dev:fixing" } }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_reviewing",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
  core.log_raise("observe_issue", proposal_id, "devloop_reviewing", reviewing_payload)
end

local function raise_review_meta_replay(issue, proposal_id, state, link, snapshot)
  local fact = core.review_meta_replay_fact(snapshot.comments, proposal_id, state.version)
  if fact == nil then
    core.log_cas_decision("observe_issue", proposal_id, state, "review-meta", "review-meta", "skip-foreign(review-meta)", "review-meta recovery facts are not visible")
    return
  end
  local payload = core.build_devloop_review_meta_payload(fact, proposal_id, state.version, fact.pr_number, fact.n, fact.source_ref)
  core.log_apply("observe_issue", proposal_id, "review-meta", state.version, { add = {}, remove = {} }, {
    "devloop_review_meta",
  })
  core.log_raise("observe_issue", proposal_id, "devloop_review_meta", payload)
end

function pipeline(event)
  local issue = event.payload or {}
  if not core.is_supported_issue(issue) then
    core.log_entry("observe_issue", event, "unknown", issue.dedup_key)
    core.log_cas_decision("observe_issue", "unknown", { state = nil, version = nil }, "unmanaged", "thinking", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  local proposal_id = core.proposal_id(issue.repo, issue.number)
  core.log_entry("observe_issue", event, proposal_id, issue.dedup_key)
  local lock_key = core.observe_lock_key(issue.repo, issue.number)
  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local state_view = exec_sync({ cmd = core.gh_issue_view_state_cmd(issue.repo, issue.number), timeout = 30 })
    if state_view.exit_code ~= 0 then
      error("github-devloop: gh issue state view failed: " .. tostring(state_view.stderr))
    end

    local current = core.parse_issue_view_state(state_view.stdout)
    if current.state ~= "OPEN" then
      core.log_cas_decision("observe_issue", proposal_id, { state = nil, version = nil }, "unmanaged", "thinking", "skip-advanced-or-diverged", "issue is not open")
      return
    end
    if not core.is_opted_in(current.labels) then
      core.log_cas_decision("observe_issue", proposal_id, { state = nil, version = nil }, "unmanaged", "thinking", "skip-not-opted-in", "fkst-dev:enabled label is absent")
      return
    end
    core.log_forged_markers("observe_issue", proposal_id, current.comments)
    local link = core.pr_link_fact(current.comments, proposal_id)
    local snapshot = core.linked_entity_snapshot(issue.repo, proposal_id, current.comments)
    local state = snapshot.state
    if state.state ~= nil then
      if state.state == "thinking" then
        core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", "skip-idempotent(already at to_state)", "trusted thinking state marker is already visible")
        raise_thinking_replay(issue, proposal_id, state, current, event.ts)
      end
      if not core.state_label_hint_matches(current.labels, state.state) then
        local label_request = core.build_reconcile_state_label_request(issue.repo, issue.number, proposal_id, state.state, state.version, issue.source_ref)
        local add_labels, remove_labels = core.state_label_changes(state.state)
        core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      if state.state == "ready" then
        local ready_payload = core.build_devloop_ready_payload({
          proposal_id = proposal_id,
          dedup_key = state.version,
          source_ref = issue.source_ref,
        })
        local gate = core.dependency_gate(issue.repo, issue.number)
        if not gate.ok then
          local marker = gate.kind == "cycle"
            and core.dependency_cycle_marker(proposal_id, state.version)
            or core.dependency_wait_marker(proposal_id, state.version, gate.unmet)
          core.log_cas_decision("observe_issue", proposal_id, state, "ready", "implementing", "hold-dependency", gate.reason)
          core.log_apply("observe_issue", proposal_id, nil, nil, { add = { core._blocked_on_dependency_label }, remove = {} }, {
            "github-proxy.github_issue_comment_request",
            "github-proxy.github_issue_label_request",
          })
          core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", {
            schema = "github-proxy.v1",
            repo = issue.repo,
            issue_number = issue.number,
            body = "github-devloop dependency hold: " .. tostring(gate.kind) .. "\n\nReason: " .. tostring(gate.reason) .. "\n\n" .. marker,
            dedup_key = core._dedup_key({ "dependency", "comment", tostring(proposal_id), tostring(state.version), tostring(gate.kind) }),
            source_ref = core.normalize_source_ref(issue.source_ref),
          })
          core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", core.build_label_request(
            issue.repo,
            issue.number,
            { core._blocked_on_dependency_label },
            {},
            core._dedup_key({ "dependency", "label", "hold", tostring(proposal_id), tostring(state.version), tostring(gate.kind) }),
            issue.source_ref
          ))
          return
        end
        local raised = { "devloop_ready" }
        if core.has_label(current.labels, core._blocked_on_dependency_label) then
          table.insert(raised, "github-proxy.github_issue_label_request")
        end
        core.log_apply("observe_issue", proposal_id, nil, nil, { add = {}, remove = { core._blocked_on_dependency_label } }, raised)
        if core.has_label(current.labels, core._blocked_on_dependency_label) then
          core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", core.build_label_request(
            issue.repo,
            issue.number,
            {},
            { core._blocked_on_dependency_label },
            core._dedup_key({ "dependency", "label", "clear", tostring(proposal_id), tostring(state.version) }),
            issue.source_ref
          ))
        end
        core.log_raise("observe_issue", proposal_id, "devloop_ready", ready_payload)
      end
      if state.state == "fixing" then
        raise_fixing_replay(issue, proposal_id, state, link, snapshot)
      end
      if state.state == "review-meta" then
        raise_review_meta_replay(issue, proposal_id, state, link, snapshot)
      end
      if state.state == "thinking" or state.state == "pr-open" then
        if state.state == "pr-open" and tostring(state.version or "") == tostring(link and link.impl_version or "") then
          raise_pr_open_reviewing(issue, proposal_id, state, link, snapshot)
        end
        return
      end
    end
    local transition = core.versioned_transition_status(state, { "unmanaged" }, "thinking", issue.dedup_key)
    if transition == "stale" then
      core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "current marker is not an unmanaged start")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "unmanaged state marker pending for observe")
      error("github-devloop: unmanaged state marker pending for observe; retrying")
    end
    core.log_cas_decision("observe_issue", proposal_id, state, "unmanaged", "thinking", core.cas_outcome(state, transition, issue.dedup_key), "starting consensus for opted-in issue")

    issue.content_fetch = core.context_fetch_ref_from_bundle({
      dept = "observe_issue",
      repo = issue.repo,
      issue_number = issue.number,
      proposal_id = proposal_id,
      version = issue.dedup_key,
      tick = event.ts,
    })
    local proposal = core.build_board_proposal(issue, event.ts)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=observe_issue proposal_id=" .. tostring(proposal_id) .. " tag=SKIP reason=cannot-build-valid-proposal")
      return
    end

    local comment_request = core.build_observe_comment_request(issue, proposal)
    local label_request = core.build_thinking_label_request(issue, proposal)
    local add_labels, remove_labels = core.state_label_changes("thinking")
    core.log_apply("observe_issue", proposal_id, "thinking", proposal.dedup_key, { add = add_labels, remove = remove_labels }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    })
    core.log_raise("observe_issue", proposal_id, "consensus.proposal", proposal)
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
  end)
end

return M
