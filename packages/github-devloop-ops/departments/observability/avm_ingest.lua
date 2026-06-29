local M = {}

local function has_entries(value)
  if type(value) ~= "table" then
    return false
  end
  for _, _ in pairs(value) do
    return true
  end
  return false
end

local function safe_segment(value)
  local text = tostring(value or "")
  text = text:gsub("[^%w%._%-/]", "-")
  if text == "" then
    return "unknown"
  end
  if #text > 120 then
    return text:sub(1, 120)
  end
  return text
end

local function marker_attr(marker, name)
  return tostring(marker or ""):match(tostring(name) .. '="([^"]*)"')
end

local function fact_from_marker(core, marker, comment)
  local proposal_id = marker_attr(marker, "proposal")
  local pr_number = marker_attr(marker, "pr")
  local version = marker_attr(marker, "version")
  local head_sha = marker_attr(marker, "head_sha")
  if proposal_id == nil or pr_number == nil or version == nil or head_sha == nil then
    return nil, "missing_identity"
  end
  return core.autonomy_result_record_from_marker(marker, comment, proposal_id, pr_number, version, head_sha)
end

function M.marker_trust_set(core)
  local managed = core.managed_bot_logins and core.managed_bot_logins() or nil
  if has_entries(managed) then
    return managed
  end
  local trusted = core.trusted_bot_login and core.trusted_bot_login() or nil
  if trusted == nil or trusted == "" then
    return {}
  end
  return { [core.strip_bot_login_suffix(trusted)] = true }
end

function M.is_trusted_comment(core, comment, trust_set)
  local author = core.comment_author_login(comment)
  if author == nil or author == "" then
    return false
  end
  return trust_set[core.strip_bot_login_suffix(author)] == true
end

function M.log_marker_rejection(core, reason, comment, marker)
  log.warn("github-devloop dept=observability tag=AVM_MARKER_REJECTED"
    .. " reason=" .. safe_segment(reason)
    .. " author=" .. safe_segment(core.comment_author_login(comment))
    .. " proposal=" .. safe_segment(marker_attr(marker, "proposal"))
    .. " pr=" .. safe_segment(marker_attr(marker, "pr"))
    .. " version=" .. safe_segment(marker_attr(marker, "version")))
end

function M.log_comment_rejection(core, reason, comment)
  log.warn("github-devloop dept=observability tag=AVM_MARKER_COMMENT_REJECTED"
    .. " reason=" .. safe_segment(reason)
    .. " author=" .. safe_segment(core.comment_author_login(comment)))
end

function M.append_comment_facts(core, facts, comments, now_seconds, decorate)
  local trust_set = M.marker_trust_set(core)
  for _, comment in ipairs(comments or {}) do
    local body = core._comment_body(comment)
    local has_avm_marker = body:find("fkst:github-devloop:autonomy-result:v1", 1, true) ~= nil
      or (body:find("fkst:github-devloop:merged:v1", 1, true) ~= nil
        and body:find('autonomy_result="v1"', 1, true) ~= nil)
    if not M.is_trusted_comment(core, comment, trust_set) then
      if has_avm_marker then
        M.log_comment_rejection(core, "untrusted_author", comment)
      end
      goto continue_comment
    end
    for marker in body:gmatch("<!%-%- fkst:github%-devloop:autonomy%-result:v1.-%-%->") do
      local fact, reason = fact_from_marker(core, marker, comment)
      if fact ~= nil then
        table.insert(facts, decorate(fact, comments, now_seconds))
      else
        M.log_marker_rejection(core, reason or "parse_nil", comment, marker)
      end
    end
    for marker in body:gmatch("<!%-%- fkst:github%-devloop:merged:v1.-%-%->") do
      if marker:find('autonomy_result="v1"', 1, true) ~= nil then
        local fact, reason = fact_from_marker(core, marker, comment)
        if fact ~= nil then
          table.insert(facts, decorate(fact, comments, now_seconds))
        else
          M.log_marker_rejection(core, reason or "parse_nil", comment, marker)
        end
      end
    end
    ::continue_comment::
  end
end

return M
