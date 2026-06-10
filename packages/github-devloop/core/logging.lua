local S = {}

function S.install(M)

local function normalized_error_message(value)
  local text = M._one_line(value or ""):lower()
  text = text:gsub("%d%d%d%d%-%d%d%-%d%d[tT ]%d%d:%d%d:%d%d%.?%d*Z?", "<time>")
  text = text:gsub("%f[%x]%x%x%x%x%x%x[%x]+%f[^%x]", "<sha>")
  text = text:gsub("/tmp/[^%s]+", "<path>")
  text = text:gsub("/var/folders/[^%s]+", "<path>")
  text = text:gsub("%s+", " ")
  return text
end

local function stable_hash(value)
  local hash = 5381
  for index = 1, #value do
    hash = (hash * 33 + value:byte(index)) % 2147483647
  end
  return "fp-" .. tostring(hash)
end

local function source_ref_field(source_ref)
  if type(source_ref) == "table" then
    return M._one_line(source_ref.kind) .. ":" .. M._one_line(source_ref.ref)
  end
  if source_ref ~= nil then
    return M._one_line(source_ref)
  end
  return nil
end

function M.error_fingerprint(error_class, queue, dept, message)
  return stable_hash(table.concat({
    tostring(error_class or "unknown-error"),
    tostring(queue or ""),
    tostring(dept or ""),
    normalized_error_message(message),
  }, "|"))
end

function M.error_fact_fields(error_class, queue, dept, message, context)
  local fields = {
    "error_class=" .. M._one_line(error_class or "unknown-error"),
    "fingerprint=" .. M.error_fingerprint(error_class, queue, dept, message),
  }
  local source_ref = source_ref_field(context and context.source_ref)
  if source_ref ~= nil and source_ref ~= "" then
    table.insert(fields, "source_ref=" .. source_ref)
  end
  if context and context.attempt ~= nil then
    table.insert(fields, "attempt=" .. M._one_line(context.attempt))
  end
  if context and context.terminal ~= nil then
    table.insert(fields, "terminal=" .. tostring(context.terminal == true))
  end
  return fields
end

function M.error_class_from_message(message)
  local text = tostring(message or "")
  if text:match("github%-devloop: .-codex failed:") then
    return "codex-failed"
  end
  local class = text:match("github%-devloop: [^:]+ failed: ([%w%-]+):")
    or text:match("github%-devloop: ([%w%-]+):")
    or text:match("github%-devloop: ([%w%-]+) failed:")
    or text:match("github%-devloop: ([%w%-]+) retrying")
  return class or "caught-failure"
end

function M.log_error_fact(level, dept, proposal_id, tag, error_class, queue, message, context)
  local fields = M.error_fact_fields(error_class, queue, dept, message, context)
  table.insert(fields, "queue=" .. M._one_line(queue))
  table.insert(fields, "error=" .. M._one_line(message))
  M.log_line(level or "error", dept, proposal_id, tag or "FAILURE", fields)
end

local function event_source_ref(event)
  if type(event) == "table" and event.source_ref ~= nil then
    return event.source_ref
  end
  local payload = type(event) == "table" and event.payload or nil
  if type(payload) == "table" then
    return payload.source_ref
  end
  return nil
end

function M.wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, err = pcall(fn, event)
    if ok then
      return err
    end
    local payload = type(event) == "table" and event.payload or nil
    local proposal_id = type(payload) == "table" and payload.proposal_id or "unknown"
    M.log_error_fact("error", dept, proposal_id, "FAILURE", M.error_class_from_message(err), type(event) == "table" and event.queue or nil, err, {
      source_ref = event_source_ref(event),
      attempt = type(event) == "table" and event.attempt or nil,
    })
    error(err, 0)
  end
end

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
    "version=" .. tostring(dedup_key or ""),
    "dedup_key=" .. tostring(dedup_key or ""),
  })
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

function M.log_codex_result(dept, proposal_id, role, result, parsed, failure, context)
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
    for _, field in ipairs(M.error_fact_fields(
      context and context.error_class or "codex-failed",
      context and context.queue,
      dept,
      failure,
      context
    )) do
      table.insert(fields, field)
    end
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
