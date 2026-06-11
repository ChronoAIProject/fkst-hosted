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

local function raise_stale_dependency_label_clear(issue, proposal_id, state, labels)
  if state.state == "ready" or not core.has_label(labels, core._blocked_on_dependency_label) then
    return false
  end
  core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = {}, remove = { core._blocked_on_dependency_label } }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", core.build_label_request(
    issue.repo,
    issue.number,
    {},
    { core._blocked_on_dependency_label },
    core._dedup_key({ "dependency", "label", "clear", tostring(proposal_id), tostring(state.version or "unversioned") }),
    issue.source_ref
  ))
  return true
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
        if core.version_loop_round(state.version) == 0 then
          issue.content_fetch = core.context_fetch_ref_from_bundle({
            dept = "observe_issue",
            repo = issue.repo,
            issue_number = issue.number,
            proposal_id = proposal_id,
            version = state.version,
            tick = event.ts,
          })
          local proposal = core.build_board_proposal(issue, event.ts)
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
          -- Converge markers store bounded digests, not the full narrowed question, so
          -- observe self-heal cannot reconstruct the exact mid-loop proposal.
        end
      end
      if not core.state_label_hint_matches(current.labels, state.state) then
        local label_request = core.build_reconcile_state_label_request(issue.repo, issue.number, proposal_id, state.state, state.version, issue.source_ref)
        local add_labels, remove_labels = core.state_label_changes(state.state)
        core.log_apply("observe_issue", proposal_id, state.state, state.version, { add = add_labels, remove = remove_labels }, {
          "github-proxy.github_issue_label_request",
        })
        core.log_raise("observe_issue", proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      raise_stale_dependency_label_clear(issue, proposal_id, state, current.labels)
      if state.state == "ready" then
        local ready_payload = core.build_devloop_ready_payload({
          proposal_id = proposal_id,
          dedup_key = state.version,
          source_ref = issue.source_ref,
        })
        local dependency_hold = core.dependency_hold_fact(current.comments, proposal_id)
        local gate = core.dependency_gate(issue.repo, issue.number)
        if dependency_hold ~= nil then
          core.log_cas_decision("observe_issue", proposal_id, state, "ready", "implementing", "recheck-dependency-hold", dependency_hold.reason)
        end
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
