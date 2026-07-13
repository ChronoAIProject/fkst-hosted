local M = {}

function M.make(core, deps)
  local config = deps.config
  local devloop_base = deps.devloop_base
  local devloop_logging = deps.devloop_logging
  local entity_lib = deps.entity_lib
  local merge_batch = deps.merge_batch
  local m_mq = deps.merge_queue
  local v_merge_ready = deps.merge_ready_validator
  local payloads_builders = deps.payloads_builders
  local process_merge_ready_locked = deps.process_merge_ready_locked

  local function synthesize_merge_ready_from_queue_head(repo, head)
    if type(head) ~= "table"
      or head.proposal_id == nil
      or head.pr_number == nil
      or head.version == nil
      or head.review_proposal_id == nil
      or head.review_dedup_key == nil
      or head.head_sha == nil then
      return nil
    end
    return payloads_builders.build_devloop_merge_ready_payload(head.proposal_id, head.pr_number, head.version, {
      review_proposal_id = head.review_proposal_id,
      review_dedup_key = head.review_dedup_key,
      reviewed_head_sha = head.head_sha,
    }, entity_lib.pr_source_ref(repo, head.pr_number))
  end

  local function merge_queue_head_all(repo, base_branch)
    local head, entries = m_mq.merge_queue_head(core, repo, base_branch); return head, entries or {}
  end

  local function chain_merge_queue_if_non_empty(repo, branches, merged_pr_number)
    local next_head = merge_queue_head_all(repo, branches.integration)
    if next_head == nil then
      devloop_logging.log_line("info", "merge", "merge", "GATE", { "outcome=quiescent", "reason=merge-queue-empty-after-progress", "pass=poll" })
    else
      local payload = m_mq.merge_queue_tick_payload(repo, merged_pr_number, next_head)
      devloop_logging.log_raise("merge", tostring(next_head.proposal_id or "merge"), "devloop_merge_queue_tick", payload)
      raise("devloop_merge_queue_tick", payload)
    end
  end

  local function queue_starvation_cause_matches_entry(cause, entry)
    local cause_pr = tonumber(cause and cause.head_pr_number)
    if cause_pr == nil or type(entry) ~= "table" then
      return false
    end
    return tostring(entry.pr_number or "") == tostring(cause_pr)
      and tostring(entry.head_sha or "") == tostring(cause.head_sha or "")
      and tostring(entry.proposal_id or "") == tostring(cause.proposal_id or "")
      and tostring(entry.version or "") == tostring(cause.version or "")
  end

  local function queue_starvation_target_entry(cause, entries)
    local target = nil
    for _, entry in ipairs(entries or {}) do
      if queue_starvation_cause_matches_entry(cause, entry) then
        target = entry
        break
      end
    end
    if target == nil then
      return nil, nil, "target-not-current"
    end
    local candidate, age_minutes = m_mq.merge_queue_starvation_candidate(entries, m_mq._merge_ready_starvation_threshold_minutes, now())
    if not queue_starvation_cause_matches_entry(cause, candidate) then
      return nil, age_minutes, "target-not-aged-candidate"
    end
    return target, age_minutes, "aged-candidate"
  end

  local function process_merge_queue_tick(event)
    local cause = type(event and event.payload) == "table" and event.payload.cause or nil
    local cause_kind = type(cause) == "table" and tostring(cause.kind or "") or ""
    local repo = devloop_base.read_env("FKST_GITHUB_REPO")
    if repo == nil or repo == "" then
      devloop_logging.log_entry("merge", event, "unknown", "")
      devloop_logging.log_line("info", "merge", "unknown", "GATE", {
        "outcome=skip",
        "reason=missing-repo-config",
        "pass=poll",
      })
      return
    end
    local lock_key = entity_lib.merge_lane_lock_key(repo)
    if lock_key == nil then
      devloop_logging.log_entry("merge", event, "unknown", "")
      devloop_logging.log_line("info", "merge", "unknown", "GATE", {
        "outcome=skip",
        "reason=no-transition-lock-key",
        "pass=poll",
      })
      return
    end
    with_lock(lock_key, function()
      devloop_base.assert_trusted_bot_configured()
      local branches = config.branch_config()
      local head, entries = merge_queue_head_all(repo, branches.integration)
      if head == nil then
        devloop_logging.log_line("info", "merge", "unknown", "GATE", {
          "outcome=skip",
          "reason=merge-queue-empty",
          "pass=poll",
        })
        return
      end
      local selected = head
      if cause_kind == "queue-starvation" then
        local cause_proposal = tostring(cause and cause.proposal_id or "")
        local selected_age
        local selected_reason
        selected, selected_age, selected_reason = queue_starvation_target_entry(cause, entries)
        if selected == nil then
          devloop_logging.log_line("info", "merge", tostring(cause_proposal ~= "" and cause_proposal or head.proposal_id), "GATE", {
            "pr=" .. tostring(head.pr_number),
            "reported_pr=" .. tostring(cause and cause.head_pr_number or ""),
            "version=" .. tostring(head.version),
            "outcome=hold",
            "reason=queue-starvation-" .. tostring(selected_reason or "target-not-current"),
            "age_minutes=" .. tostring(selected_age or ""),
            "incident=" .. tostring(cause and cause.incident_identity or ""),
            "pass=poll",
          })
          return
        end
        devloop_logging.log_line("info", "merge", selected.proposal_id, "GATE", {
          "pr=" .. tostring(selected.pr_number),
          "version=" .. tostring(selected.version),
          "outcome=reconcile",
          "reason=queue-starvation-redrive",
          "age_minutes=" .. tostring(selected_age or ""),
          "incident=" .. tostring(cause.incident_identity or ""),
          "pass=poll",
        })
      end
      if selected.state == "merging" then
        devloop_logging.log_line("info", "merge", selected.proposal_id, "GATE", {
          "pr=" .. tostring(selected.pr_number),
          "version=" .. tostring(selected.version),
          "outcome=skip",
          "reason=merge-queue-head-merging",
          "pass=poll",
        })
        return
      end
      local merge_ready = synthesize_merge_ready_from_queue_head(repo, selected)
      if merge_ready == nil or not v_merge_ready.is_supported_merge_ready(merge_ready) then
        devloop_logging.log_line("info", "merge", selected.proposal_id, "GATE", {
          "pr=" .. tostring(selected.pr_number),
          "version=" .. tostring(selected.version),
          "outcome=skip",
          "reason=merge-queue-head-missing-merge-ready-fact",
          "pass=poll",
        })
        return
      end
      local entity = entity_lib.parse_entity_proposal_id(merge_ready.proposal_id)
      if entity == nil then
        devloop_logging.log_cas_decision("merge", merge_ready.proposal_id, { state = nil, version = nil }, "merge-ready", "merged|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
        return
      end
      merge_ready._merge_pass = "poll"
      devloop_logging.log_entry("merge", event, merge_ready.proposal_id, merge_ready.dedup_key)
      local selected_is_fifo_head = queue_starvation_cause_matches_entry(cause, head)
      local write_mode = config.write_mode()
      local outcome = process_merge_ready_locked(repo, entity.issue_number, merge_ready, branches, nil, {
        enforce_queue = false,
        write_mode = write_mode,
        queue_starvation_cause = cause_kind == "queue-starvation" and cause or nil,
      })
      if outcome ~= nil and outcome.status == "merged" then
        local last_merged_pr_number = outcome.pr_number
        if cause_kind ~= "queue-starvation" or selected_is_fifo_head then
          last_merged_pr_number = merge_batch.run_merge_batch_window(core, repo, branches, merge_ready, entries, { write_mode = write_mode }, process_merge_ready_locked)
        end
        chain_merge_queue_if_non_empty(repo, branches, last_merged_pr_number or outcome.pr_number)
      end
    end)
  end

  return {
    process_merge_queue_tick = process_merge_queue_tick,
  }
end

return M
