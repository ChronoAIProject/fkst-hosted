local S = {}

function S.install(M)
function M.log_line(level, dept, proposal_id, tag, fields)
  local parts = {
    "github-devloop",
    "dept=" .. tostring(dept or "unknown"),
    "proposal_id=" .. tostring(proposal_id or "unknown"),
    "tag=" .. tostring(tag or "event"),
  }
  for _, field in ipairs(fields or {}) do
    table.insert(parts, tostring(field))
  end
  log[level or "info"](table.concat(parts, " "))
end

function M.log_entry(dept, event, proposal_id, dedup_key)
  M.log_line("info", dept, proposal_id, "ENTRY", {
    "queue=" .. tostring(event and event.queue or "unknown"),
    "payload_type=" .. type(event and event.payload),
    "version=" .. tostring(dedup_key or ""),
    "dedup_key=" .. tostring(dedup_key or ""),
  })
end

function M.payload_field(payload, key)
  if type(payload) ~= "table" then
    return nil
  end
  return payload[key]
end

function M.log_cas_decision(dept, proposal_id, current, from_state, to_state, outcome, reason)
  local current_state = current
  local current_version = type(current) == "table" and current.version or nil
  if type(current) == "table" then
    current_state = current.state
  end
  M.log_line("info", dept, proposal_id, "CAS", {
    "current_state=" .. tostring(current_state or "unmanaged"),
    "current_version=" .. tostring(current_version or ""),
    "current_source=trusted-marker",
    "transition=" .. tostring(from_state or "unknown") .. "->" .. tostring(to_state or "unknown"),
    "outcome=" .. tostring(outcome or "unknown"),
    "reason=" .. M._one_line(reason or ""),
  })
end

function M.log_apply(dept, proposal_id, to_state, version, labels, events)
  local add_labels = labels and labels.add or {}
  local remove_labels = labels and labels.remove or {}
  M.log_line("info", dept, proposal_id, "APPLY", {
    "state_marker_state=" .. tostring(to_state or "none"),
    "state_marker_version=" .. tostring(version or ""),
    "set_exclusive_add=" .. table.concat(add_labels, ","),
    "set_exclusive_remove=" .. table.concat(remove_labels, ","),
    "raised=" .. table.concat(events or {}, ","),
  })
end

function M.log_outbound(dept, proposal_id, queue, request)
  M.log_line("info", dept, proposal_id, "OUTBOUND", {
    "mode=" .. M.write_mode(),
    "queue=" .. tostring(queue or ""),
    "repo=" .. tostring(request and request.repo or ""),
    "issue=" .. tostring(request and request.issue_number or ""),
    "branch=" .. tostring(request and request.branch or ""),
    "pr=" .. tostring(request and request.pr_number or ""),
    "dedup_key=" .. tostring(request and request.dedup_key or ""),
  })
end

function M.log_raise(dept, proposal_id, queue, payload)
  if queue == "github-proxy.github_issue_label_request"
    or queue == "github-proxy.github_issue_comment_request"
    or queue == "github-proxy.github_pr_comment_request"
    or queue == "github-proxy.github_issue_create_request"
    or queue == "github-proxy.github_pr_open_request" then
    M.log_outbound(dept, proposal_id, queue, payload)
  end
  raise(queue, payload)
end

function M.log_codex_start(dept, proposal_id, role)
  M.log_line("info", dept, proposal_id, "CODEX", {
    "phase=start",
    "role=" .. tostring(role or dept),
  })
end

function M.log_codex_result(dept, proposal_id, role, result, parsed, failure)
  local level = failure and "error" or "info"
  local fields = {
    "phase=result",
    "role=" .. tostring(role or dept),
    "exit_code=" .. tostring(type(result) == "table" and result.exit_code or "nil"),
  }
  if parsed ~= nil then
    table.insert(fields, "parsed=" .. M._one_line(parsed))
  end
  if failure ~= nil then
    table.insert(fields, "failure=" .. M._one_line(failure))
  end
  M.log_line(level, dept, proposal_id, "CODEX", fields)
end

function M.log_forged_markers(dept, proposal_id, comments)
  if type(comments) ~= "table" then
    return
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:([%w%-]+):v1.-%-%->"
  for _, comment in ipairs(comments) do
    if not M._is_trusted_comment(comment) then
      for marker, marker_kind in M._comment_body(comment):gmatch("(" .. marker_pattern .. ")") do
        local marker_proposal = marker:match('proposal="([^"]+)"')
        if marker_proposal == proposal_id then
          M.log_line("warn", dept, proposal_id, "FORGE", {
            "marker_kind=" .. tostring(marker_kind),
            "ignored_author=" .. tostring(M._comment_author_login(comment) or ""),
            "trusted_bot=" .. tostring(M.trusted_bot_login()),
          })
        end
      end
    end
  end
end

end

return S
