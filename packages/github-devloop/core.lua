local M = {}

local max_key_len = 200
local max_dedup_len = 512
local max_title_len = 240
local max_body_len = 12000
local max_comments_len = 12000
local max_meta_reason_len = 2000
local max_impl_output_len = 2000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50
local max_worktree_prefix_len = 90
local action_label = "⟦FKST:ACTION⟧"
local reason_label = "⟦FKST:REASON⟧"
local untrusted_issue_data_begin = "BEGIN UNTRUSTED ISSUE DATA"
local untrusted_issue_data_end = "END UNTRUSTED ISSUE DATA"
local test_bot_login = "fkst-test-bot"

local enabled_label = "fkst-dev:enabled"
local thinking_label = "fkst-dev:thinking"
local ready_label = "fkst-dev:ready"
local implementing_label = "fkst-dev:implementing"
local impl_failed_label = "fkst-dev:impl-failed"
local blocked_label = "fkst-dev:blocked"
local stuck_label = "fkst-dev:stuck"
local loop_budget = 3

local state_labels = {
  [thinking_label] = true,
  [ready_label] = true,
  [implementing_label] = true,
  [impl_failed_label] = true,
  [blocked_label] = true,
  [stuck_label] = true,
}

local label_by_state = {
  thinking = thinking_label,
  ready = ready_label,
  implementing = implementing_label,
  ["impl-failed"] = impl_failed_label,
  blocked = blocked_label,
  stuck = stuck_label,
}

local state_by_label = {}
for state, label in pairs(label_by_state) do
  state_by_label[label] = state
end

local state_graph = {
  unmanaged = { "thinking" },
  thinking = { "ready", "blocked", "stuck" },
  stuck = { "ready", "blocked" },
  ready = { "implementing" },
  implementing = { "impl-failed" },
  ["impl-failed"] = {},
  blocked = {},
}

local state_order = { "thinking", "ready", "implementing", "impl-failed", "blocked", "stuck" }
local state_stage_rank = {
  thinking = 100,
  stuck = 300,
  ready = 500,
  implementing = 600,
  ["impl-failed"] = 700,
  blocked = 800,
}
local trusted_bot_login = nil
local comment_body
local comment_author_login
local is_trusted_comment

local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_WRITE = true,
}

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function trim(value)
  return tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function neutralize_fkst_markers(value)
  local neutralized = tostring(value or ""):gsub("<!%-%- fkst:", "&lt;!-- fkst:")
  return neutralized
end

local function one_line(value)
  return tostring(value or ""):gsub("[%s]+", " ")
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_meta_action(value)
  return value == "implement" or value == "split" or value == "block"
end

local function is_path_safe_key(value, limit)
  if not is_bounded_string(value, limit or max_key_len) then
    return false
  end
  if value:sub(1, 1) == "/" then
    return false
  end
  if value:find("\\", 1, true) ~= nil then
    return false
  end
  if value:find("%s") ~= nil then
    return false
  end
  if value:find("[^%w%._%-%/#]") ~= nil then
    return false
  end
  for segment in value:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      return false
    end
  end
  return true
end

local function has_bounded_source_ref(source_ref)
  return type(source_ref) == "table"
    and is_bounded_string(source_ref.kind, max_key_len)
    and is_bounded_string(source_ref.ref, max_key_len)
end

function M.read_env_command(name)
  if not allowed_env[name] then
    error("github-devloop: env name is not allowed")
  end
  return 'printf %s "$' .. name .. '"'
end

function M.read_env(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    return nil
  end
  local ok, out = pcall(run, M.read_env_command(name))
  if not ok or type(out) ~= "table" or out.exit_code ~= 0 or out.stdout == "" then
    return nil
  end
  return out.stdout
end

function M.configure_trusted_bot_login(login)
  if login == nil or tostring(login) == "" then
    trusted_bot_login = nil
    return nil
  end
  trusted_bot_login = tostring(login)
  return trusted_bot_login
end

function M.assert_trusted_bot_configured()
  local login = M.read_env("FKST_GITHUB_BOT_LOGIN")
  if login ~= nil then
    M.configure_trusted_bot_login(login)
  end

  if M.read_env("FKST_GITHUB_WRITE") == "1" and trusted_bot_login == nil then
    error("github-devloop: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
  end
  return trusted_bot_login
end

function M.sanitize_key(value, limit)
  local max_len = max_key_len
  if limit ~= nil then
    max_len = limit
  end
  local sanitized = tostring(value or ""):gsub("[^%w%._%-%/#]", "-")
  sanitized = sanitized:gsub("/+", "/")
  sanitized = sanitized:gsub("^/+", ""):gsub("/+$", "")
  if sanitized == "" then
    return "empty"
  end

  local segments = {}
  for segment in sanitized:gmatch("[^/]+") do
    local safe_segment = segment
    if safe_segment == "." or safe_segment == ".." then
      safe_segment = "-"
    end
    table.insert(segments, safe_segment)
  end

  sanitized = table.concat(segments, "/")
  if max_len ~= false and #sanitized > max_len then
    sanitized = sanitized:sub(1, max_len)
    sanitized = sanitized:gsub("/+$", "")
  end
  if sanitized == "" then
    return "empty"
  end
  return sanitized
end

local function dedup_key(parts)
  local key = M.sanitize_key(table.concat(parts, "/"), false)
  if not is_path_safe_key(key, max_dedup_len) then
    error("github-devloop: invalid dedup_key")
  end
  return key
end

function M.safe_repo(repo)
  local safe = M.sanitize_key(repo):sub(1, max_repo_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.safe_issue(issue_number)
  local safe = M.sanitize_key(issue_number):sub(1, max_issue_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.safe_updated_at(updated_at)
  local safe = M.sanitize_key(updated_at):sub(1, max_update_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.is_opted_in(labels)
  if type(labels) ~= "table" then
    return false
  end

  for _, label in ipairs(labels) do
    if tostring(label) == enabled_label then
      return true
    end
  end
  return false
end

function M.proposal_id(repo, issue_number)
  return "github-devloop/issue/" .. M.safe_repo(repo) .. "/" .. M.safe_issue(issue_number)
end

function M.parse_proposal_id(id)
  if type(id) ~= "string" then
    return nil
  end

  local rest = id:match("^github%-devloop/issue/(.+)$")
  if rest == nil then
    return nil
  end

  local issue_number = rest:match("/([^/]+)$")
  local repo = issue_number and rest:sub(1, #rest - #issue_number - 1) or nil
  if repo == nil or repo == "" or issue_number == nil or issue_number == "" then
    return nil
  end
  return repo, issue_number
end

function M.is_safe_proposal_ref(proposal_id, dedup_key)
  if not is_path_safe_key(proposal_id, max_key_len) then
    return false
  end
  if not is_path_safe_key(dedup_key, max_dedup_len) then
    return false
  end

  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  return M.issue_ref_round_trips(repo, issue_number)
end

function M.is_safe_consensus_result_ref(proposal_id, dedup_key)
  if not is_path_safe_key(proposal_id, max_key_len) then
    return false
  end
  if not is_bounded_string(dedup_key, max_dedup_len) then
    return false
  end

  local inner_dedup_key = dedup_key:match("^consensus:(.+)$") or dedup_key
  if not is_path_safe_key(inner_dedup_key, max_dedup_len) then
    return false
  end

  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  return M.issue_ref_round_trips(repo, issue_number)
end

function M.issue_ref_round_trips(repo, issue_number)
  local repo_text = tostring(repo)
  local issue_text = tostring(issue_number)
  if M.safe_repo(repo) ~= repo_text then
    return false
  end
  if M.safe_issue(issue_number) ~= issue_text then
    return false
  end

  local parsed_repo, parsed_issue = M.parse_proposal_id(M.proposal_id(repo, issue_number))
  return parsed_repo == repo_text and parsed_issue == issue_text
end

function M.proposal_dedup_key(proposal_id, updated_at)
  return tostring(proposal_id) .. "/" .. M.safe_updated_at(updated_at)
end

function M.observe_lock_key(repo, issue_number)
  return "github-devloop/transition/" .. M.safe_repo(repo) .. "/issue/" .. M.safe_issue(issue_number)
end

function M.transition_lock_key(proposal_id)
  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil then
    return nil
  end
  return M.observe_lock_key(repo, issue_number)
end

function M.result_lock_key(proposal_id)
  return M.transition_lock_key(proposal_id)
end

function M.loop_lock_key(proposal_id)
  return M.transition_lock_key(proposal_id)
end

function M.meta_lock_key(proposal_id)
  return M.transition_lock_key(proposal_id)
end

function M.implement_lock_key(proposal_id)
  return M.transition_lock_key(proposal_id)
end

function M.safe_issue_slug(repo, issue_number)
  local slug = M.sanitize_key(tostring(repo or "") .. "-" .. tostring(issue_number or ""), false):gsub("/", "-")
  slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if slug == "" then
    slug = "issue"
  end
  if #slug > max_worktree_prefix_len then
    slug = slug:sub(1, max_worktree_prefix_len):gsub("%-+$", "")
  end
  if slug == "" then
    return "issue"
  end
  return slug
end

function M.bounded_body(value)
  local text = tostring(value or "")
  if text == "" then
    return "(empty issue body)"
  end
  if #text <= max_body_len then
    return text
  end
  return text:sub(1, max_body_len)
end

function M.max_body_len()
  return max_body_len
end

function M.render_template(template, vars)
  if type(template) ~= "string" then
    error("github-devloop: template must be a string")
  end
  if type(vars) ~= "table" then
    error("github-devloop: template vars must be a table")
  end

  return (template:gsub("{{([%w_]+)}}", function(name)
    local value = vars[name]
    if value == nil then
      error("github-devloop: missing template var " .. name)
    end
    return tostring(value)
  end))
end

function M.neutralize_untrusted_prompt_text(text)
  local value = tostring(text or "")

  local function neutralize_line(line)
    if line:match("^%s*" .. action_label) ~= nil
      or line:match("^%s*" .. reason_label) ~= nil
      or trim(line) == untrusted_issue_data_begin
      or trim(line) == untrusted_issue_data_end then
      return "> " .. line
    end
    return line
  end

  local output = {}
  local start = 1
  while true do
    local newline = value:find("\n", start, true)
    if newline == nil then
      table.insert(output, neutralize_line(value:sub(start)))
      break
    end

    table.insert(output, neutralize_line(value:sub(start, newline - 1)))
    table.insert(output, "\n")
    start = newline + 1
  end

  return table.concat(output)
end

function M.neutralize_untrusted_comment_text(text)
  local value = tostring(text or "")

  local function neutralize_line(line)
    if line:find("<!-- fkst:", 1, true) ~= nil then
      return neutralize_fkst_markers(line)
    end
    return line
  end

  local output = {}
  local start = 1
  while true do
    local newline = value:find("\n", start, true)
    if newline == nil then
      table.insert(output, neutralize_line(value:sub(start)))
      break
    end

    table.insert(output, neutralize_line(value:sub(start, newline - 1)))
    table.insert(output, "\n")
    start = newline + 1
  end

  return table.concat(output)
end

function M.normalize_source_ref(source_ref)
  if not has_bounded_source_ref(source_ref) then
    error("github-devloop: invalid source_ref")
  end
  return {
    kind = source_ref.kind,
    ref = source_ref.ref,
  }
end

function M.gh_issue_view_body_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json body"
end

function M.gh_issue_view_state_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,state,comments"
end

function M.gh_issue_view_result_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.gh_issue_view_loop_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,updatedAt,labels,comments,state"
end

function M.gh_issue_view_meta_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_implement_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.git_status_cmd(worktree)
  return "git -C " .. shell_single_quote(worktree) .. " status --porcelain"
end

function M.parse_issue_view_body(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.bounded_body(decoded.body)
end

function M.parse_issue_view_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.issue_state_from_json(decoded)
end

function M.issue_state_from_json(decoded)
  local labels = {}
  for _, label in ipairs(decoded.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end

  return {
    labels = labels,
    comments = M.comments_from_json(decoded.comments),
    state = decoded.state,
  }
end

function M.comments_from_json(comments_json)
  local comments = {}
  for _, comment in ipairs(comments_json or {}) do
    if type(comment) == "table" and comment.body ~= nil then
      local author_login = nil
      if type(comment.author) == "table" and comment.author.login ~= nil then
        author_login = tostring(comment.author.login)
      elseif comment.author_login ~= nil then
        author_login = tostring(comment.author_login)
      end
      table.insert(comments, {
        body = tostring(comment.body),
        author_login = author_login,
      })
    elseif type(comment) == "string" then
      table.insert(comments, {
        body = comment,
        author_login = test_bot_login,
      })
    end
  end
  return comments
end

function M.parse_issue_view_result(stdout)
  local decoded = json.decode(stdout or "{}")
  local state = M.issue_state_from_json(decoded)

  return {
    labels = state.labels,
    comments = state.comments,
  }
end

function M.parse_issue_view_loop(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = M.bounded_body(decoded.body),
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_meta(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = M.bounded_body(decoded.body),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_implement(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.has_label(labels, expected)
  if type(labels) ~= "table" then
    return false
  end
  for _, label in ipairs(labels) do
    if tostring(label) == expected then
      return true
    end
  end
  return false
end

function M.state_label(state)
  return label_by_state[state]
end

function M.state_marker(proposal_id, state, version)
  if state ~= "thinking"
    and state ~= "ready"
    and state ~= "implementing"
    and state ~= "impl-failed"
    and state ~= "blocked"
    and state ~= "stuck" then
    error("github-devloop: invalid state")
  end
  return '<!-- fkst:github-devloop:state:v1 proposal="' .. tostring(proposal_id)
    .. '" state="' .. tostring(state)
    .. '" version="' .. tostring(version)
    .. '" stage_rank="' .. tostring(state_stage_rank[state])
    .. '" -->'
end

function comment_body(comment)
  if type(comment) == "table" then
    return tostring(comment.body or "")
  end
  return tostring(comment or "")
end

function comment_author_login(comment)
  if type(comment) == "table" then
    return comment.author_login
  end
  return test_bot_login
end

function is_trusted_comment(comment)
  return comment_author_login(comment) == (trusted_bot_login or test_bot_login)
end

local function trusted_marker_comments(comments)
  local filtered = {}
  if type(comments) ~= "table" then
    return filtered
  end
  for _, comment in ipairs(comments) do
    if is_trusted_comment(comment) then
      table.insert(filtered, comment)
    end
  end
  return filtered
end

function M.comment_body(comment)
  return comment_body(comment)
end

function M.comment_author_login(comment)
  return comment_author_login(comment)
end

function M.trusted_bot_login()
  return trusted_bot_login or test_bot_login
end

function M.write_mode()
  return M.read_env("FKST_GITHUB_WRITE") == "1" and "real" or "dry-run"
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
    "reason=" .. one_line(reason or ""),
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
    "dedup_key=" .. tostring(request and request.dedup_key or ""),
  })
end

function M.log_raise(dept, proposal_id, queue, payload)
  if queue == "github-proxy.github_issue_label_request"
    or queue == "github-proxy.github_issue_comment_request" then
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
    table.insert(fields, "parsed=" .. one_line(parsed))
  end
  if failure ~= nil then
    table.insert(fields, "failure=" .. one_line(failure))
  end
  M.log_line(level, dept, proposal_id, "CODEX", fields)
end

function M.log_forged_markers(dept, proposal_id, comments)
  if type(comments) ~= "table" then
    return
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:([%w%-]+):v1.-%-%->"
  for _, comment in ipairs(comments) do
    if not is_trusted_comment(comment) then
      for marker, marker_kind in comment_body(comment):gmatch("(" .. marker_pattern .. ")") do
        local marker_proposal = marker:match('proposal="([^"]+)"')
        if marker_proposal == proposal_id then
          M.log_line("warn", dept, proposal_id, "FORGE", {
            "marker_kind=" .. tostring(marker_kind),
            "ignored_author=" .. tostring(comment_author_login(comment) or ""),
            "trusted_bot=" .. tostring(M.trusted_bot_login()),
          })
        end
      end
    end
  end
end

function M.version_order_key(version)
  local text = tostring(version or "")
  local rest = text
  if rest:sub(1, #"consensus:") == "consensus:" then
    rest = rest:sub(#"consensus:" + 1)
  elseif rest:sub(1, #"ready/") == "ready/" then
    rest = rest:sub(#"ready/" + 1):gsub("^consensus%-", "")
  end

  local timestamp = nil
  for found in rest:gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    timestamp = found
  end
  if timestamp ~= nil then
    local _, end_pos = rest:find(timestamp, 1, true)
    local suffix = end_pos and rest:sub(end_pos + 1) or ""
    local loop_n = tonumber(suffix:match("/loop/(%d+)$")) or 0
    local suffix_tie = suffix:gsub("/loop/%d+$", "")
    return timestamp:gsub(":", "-") .. "/loop/" .. string.format("%012d", loop_n) .. suffix_tie
  end
  return rest
end

function M.stage_rank(state)
  return state_stage_rank[state] or 0
end

function M.version_updated_at(version)
  local text = tostring(version or "")
  local updated_at = ""
  for found in text:gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    updated_at = found:gsub(":", "-")
  end
  return updated_at
end

function M.version_loop_round(version)
  local n = tostring(version or ""):match("/loop/(%d+)$")
  return tonumber(n) or 0
end

local function version_primary_key(version)
  local updated_at = M.version_updated_at(version)
  if updated_at ~= "" then
    return updated_at
  end
  return M.version_order_key(version)
end

local function version_sort_key(version, stage_rank)
  return {
    primary = version_primary_key(version),
    loop_n = M.version_loop_round(version),
    stage_rank = tonumber(stage_rank) or 0,
  }
end

local function marker_stage_rank(marker, state)
  local explicit_rank = tonumber(marker:match('stage_rank="(%d+)"'))
  return explicit_rank or M.stage_rank(state)
end

local function compare_state_marker(a, b)
  if a == nil then
    return true
  end
  local a_key = version_sort_key(a.version, a.stage_rank)
  local b_key = version_sort_key(b.version, b.stage_rank)
  if b_key.primary ~= a_key.primary then
    return b_key.primary > a_key.primary
  end
  if b_key.loop_n ~= a_key.loop_n then
    return b_key.loop_n > a_key.loop_n
  end
  if a.version == b.version
    and ((a.state == "ready" and b.state == "blocked") or (a.state == "blocked" and b.state == "ready")) then
    return b.state == "blocked"
  end
  if b_key.stage_rank ~= a_key.stage_rank then
    return b_key.stage_rank > a_key.stage_rank
  end
  return false
end

function M.comment_bodies(comments)
  local bodies = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(bodies, comment_body(comment))
  end
  return bodies
end

function M.current_state(comments, proposal_id)
  if type(comments) ~= "table" then
    return nil
  end

  local current = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_state = marker:match('state="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      if marker_proposal == proposal_id and label_by_state[marker_state] ~= nil then
        local candidate = {
          state = marker_state,
          version = marker_version,
          stage_rank = marker_stage_rank(marker, marker_state),
        }
        if compare_state_marker(current, candidate) then
          current = candidate
        end
      end
    end
  end
  return current or {
    state = nil,
    version = nil,
    stage_rank = 0,
  }
end

function M.has_state_marker(comments, proposal_id, state, version)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.state_marker(proposal_id, state, version)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function normalize_state(state)
  if state == nil then
    return "unmanaged"
  end
  return state
end

local function can_reach(from_state, to_state, seen)
  local from = normalize_state(from_state)
  if from == to_state then
    return true
  end
  if state_graph[from] == nil then
    return false
  end
  local visited = seen or {}
  if visited[from] then
    return false
  end
  visited[from] = true
  for _, next_state in ipairs(state_graph[from]) do
    if can_reach(next_state, to_state, visited) then
      return true
    end
  end
  return false
end

function M.transition_status(current, from_states, to_state)
  local current_state = current
  if type(current) == "table" then
    current_state = current.state
  end
  if current_state == to_state then
    return "idempotent"
  end
  local normalized_current = normalize_state(current_state)
  for _, from_state in ipairs(from_states or {}) do
    if normalized_current == normalize_state(from_state) then
      return "apply"
    end
  end
  for _, from_state in ipairs(from_states or {}) do
    if can_reach(normalized_current, normalize_state(from_state)) then
      return "pending"
    end
  end
  return "stale"
end

function M.versioned_transition_status(current, from_states, to_state, incoming_version)
  if type(current) == "table"
    and current.version ~= nil
    and incoming_version ~= nil
    and M.version_order_key(incoming_version) < M.version_order_key(current.version) then
    return "stale"
  end
  local status = M.transition_status(current, from_states, to_state)
  return status
end

function M.cas_outcome(current, transition, incoming_version)
  if transition == "apply" then
    return "applied"
  end
  if transition == "idempotent" then
    return "skip-idempotent(already at to_state)"
  end
  if transition == "pending" then
    return "retry-pending(from-state marker not yet visible)"
  end
  if transition == "stale" then
    if type(current) == "table"
      and current.version ~= nil
      and incoming_version ~= nil
      and M.version_order_key(incoming_version) < M.version_order_key(current.version) then
      return "skip-stale(incoming version < current marker version)"
    end
    return "skip-advanced-or-diverged"
  end
  return tostring(transition or "unknown")
end

function M.state_label_changes(to_state)
  local add_label = M.state_label(to_state)
  if add_label == nil then
    error("github-devloop: invalid state")
  end

  local remove_labels = {}
  for _, state in ipairs(state_order) do
    local label = label_by_state[state]
    if state ~= to_state then
      table.insert(remove_labels, label)
    end
  end
  return { add_label }, remove_labels
end

function M.has_terminal_label(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, impl_failed_label)
    or M.has_label(labels, blocked_label)
    or M.has_label(labels, stuck_label)
end

function M.has_thinking_label(labels)
  return M.has_label(labels, thinking_label)
end

function M.has_stuck_label(labels)
  return M.has_label(labels, stuck_label)
end

function M.has_blocked_label(labels)
  return M.has_label(labels, blocked_label)
end

function M.has_ready_label(labels)
  return M.has_label(labels, ready_label)
end

function M.has_implementing_label(labels)
  return M.has_label(labels, implementing_label)
end

function M.has_impl_failed_label(labels)
  return M.has_label(labels, impl_failed_label)
end

function M.has_decision_terminal_label(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, impl_failed_label)
    or M.has_label(labels, blocked_label)
end

function M.is_loop_terminal(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, impl_failed_label)
    or M.has_label(labels, blocked_label)
    or M.has_label(labels, stuck_label)
end

function M.has_result_marker(comments, proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  -- Match the FULL marker (proposal + decision + dedup) so a stale opposite/older-version marker
  -- does not suppress writing the current decision's result marker.
  local needle = M.result_marker(proposal_id, decision, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.loop_budget()
  return loop_budget
end

function M.loop_marker(proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:loop:v1 proposal="' .. tostring(proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.stuck_marker(proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:stuck:v1 proposal="' .. tostring(proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.meta_marker(proposal_id, dedup_key)
  return '<!-- fkst:github-devloop:meta:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.implementing_marker(proposal_id, dedup_key)
  return '<!-- fkst:github-devloop:implementing:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.impl_failure_marker(proposal_id, dedup_key, reason)
  local safe_reason = M.sanitize_key(reason or "failed"):gsub("/", "-")
  return '<!-- fkst:github-devloop:impl-failure:v1 proposal="' .. tostring(proposal_id)
    .. '" reason="' .. safe_reason
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

local function marker_records(comments, kind, proposal_id)
  local records = {}
  if type(comments) ~= "table" then
    return records
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. kind .. ":v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local n = tonumber(marker:match('n="(%d+)"'))
      local dedup_key = marker:match('dedup="([^"]*)"')
      if marker_proposal == proposal_id and n ~= nil then
        table.insert(records, {
          n = n,
          dedup_key = dedup_key,
        })
      end
    end
  end
  return records
end

local function has_marker_round(comments, kind, proposal_id, n)
  for _, record in ipairs(marker_records(comments, kind, proposal_id)) do
    if record.n == n then
      return true
    end
  end
  return false
end

function M.has_loop_marker(comments, proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.loop_marker(proposal_id, n, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_loop_marker_round(comments, proposal_id, n)
  return has_marker_round(comments, "loop", proposal_id, n)
end

function M.has_loop_marker_dedup(comments, proposal_id, dedup_key)
  for _, record in ipairs(marker_records(comments, "loop", proposal_id)) do
    if record.dedup_key == tostring(dedup_key) then
      return true
    end
  end
  return false
end

function M.has_stuck_marker(comments, proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.stuck_marker(proposal_id, n, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_stuck_marker_round(comments, proposal_id, n)
  return has_marker_round(comments, "stuck", proposal_id, n)
end

function M.loop_count_from_github_markers(comments, proposal_id)
  local max_n = 0
  for _, record in ipairs(marker_records(comments, "loop", proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  for _, record in ipairs(marker_records(comments, "stuck", proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  return max_n
end

function M.has_meta_marker(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:meta:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == proposal_id and marker_dedup == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

local function has_versioned_marker(comments, marker)
  if type(comments) ~= "table" then
    return false
  end
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_implementing_marker(comments, proposal_id, dedup_key)
  return has_versioned_marker(comments, M.implementing_marker(proposal_id, dedup_key))
end

function M.has_impl_failure_marker(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:impl%-failure:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == proposal_id and marker_dedup == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.has_implementation_fact_marker(comments, proposal_id, dedup_key)
  return M.has_implementing_marker(comments, proposal_id, dedup_key)
    or M.has_impl_failure_marker(comments, proposal_id, dedup_key)
end

function M.has_no_consensus_stuck_fact(comments, proposal_id, dedup_key)
  local budget = M.loop_budget()
  return M.has_stuck_marker(comments, proposal_id, budget, dedup_key)
    or M.has_loop_marker_dedup(comments, proposal_id, dedup_key)
end

function M.parse_loop_round_from_dedup(dedup_key)
  local n = tostring(dedup_key or ""):match("/loop/(%d+)$")
  return tonumber(n) or 0
end

function M.build_devloop_stuck_payload(unresolved, n)
  return {
    schema = "github-devloop.stuck.v1",
    proposal_id = unresolved.proposal_id,
    dedup_key = dedup_key({
      tostring(unresolved.proposal_id),
      "stuck",
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    no_consensus_dedup_key = unresolved.dedup_key,
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.build_devloop_ready_payload(source)
  return {
    schema = "github-devloop.ready.v1",
    proposal_id = source.proposal_id,
    dedup_key = dedup_key({
      "ready",
      tostring(source.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source.source_ref),
  }
end

function M.build_meta_prompt(proposal_id, current)
  local prompt = require("prompts.meta")
  local comments = table.concat(M.comment_bodies(current.comments), "\n\n--- comment ---\n\n")
  if #comments > max_comments_len then
    comments = comments:sub(1, max_comments_len)
  end

  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(proposal_id),
    title = M.neutralize_untrusted_prompt_text(current.title),
    body = M.neutralize_untrusted_prompt_text(current.body),
    comments = M.neutralize_untrusted_prompt_text(comments),
  })
end

function M.build_implement_prompt(proposal_id, current)
  local prompt = require("prompts.implement")
  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(proposal_id),
    title = M.neutralize_untrusted_prompt_text(current.title),
    body = M.neutralize_untrusted_prompt_text(current.body),
  })
end

function M.parse_meta_action(stdout)
  local text = tostring(stdout or "")

  local action = nil
  local action_count = 0
  local action_index = nil
  local reason = nil
  local reason_count = 0
  local reason_index = nil
  local index = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    index = index + 1

    -- Any line that STARTS with a sentinel must be a clean, well-formed line; a malformed
    -- sentinel-start line (extra words / junk / non-whitelisted / empty) fails the whole parse
    -- closed, so a valid pair followed by a malformed sentinel cannot be silently accepted.
    if line:match("^%s*" .. action_label) ~= nil then
      local token = line:match("^%s*" .. action_label .. "%s*(%a+)%s*$")
      if token == nil or not is_meta_action(token:lower()) then
        return nil
      end
      action = token:lower()
      action_count = action_count + 1
      action_index = index
    end

    if line:match("^%s*" .. reason_label) ~= nil then
      local captured = line:match("^%s*" .. reason_label .. "%s*(.+)$")
      if captured == nil or trim(captured) == "" then
        return nil
      end
      reason = trim(captured)
      reason_count = reason_count + 1
      reason_index = index
    end
  end

  if action_count ~= 1 or reason_count ~= 1 then
    return nil
  end
  if action == nil or reason == nil then
    return nil
  end
  if reason_index ~= action_index + 1 then
    return nil
  end
  if not is_bounded_string(reason, max_meta_reason_len) then
    return nil
  end

  return {
    action = action,
    reason = reason,
  }
end

function M.build_proposal(issue, body)
  local proposal_id = M.proposal_id(issue.repo, issue.number)
  local title = tostring(issue.title or "")
  if #title > max_title_len then
    title = title:sub(1, max_title_len)
  end

  return {
    schema = "consensus.proposal.v1",
    proposal_id = proposal_id,
    title = title,
    body = M.bounded_body(body),
    dedup_key = M.proposal_dedup_key(proposal_id, issue.updated_at),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_loop_proposal(repo, issue_number, current, source_ref, n)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = source_ref,
  }
  local proposal = M.build_proposal(issue, current.body)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return proposal
end

function M.validate_proposal(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(proposal.proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  if not M.is_safe_proposal_ref(proposal.proposal_id, proposal.dedup_key) then
    return false
  end
  if not is_bounded_string(proposal.title, max_title_len) then
    return false
  end
  if not is_bounded_string(proposal.body, max_body_len) then
    return false
  end
  return has_bounded_source_ref(proposal.source_ref)
end

function M.result_marker(proposal_id, decision, dedup_key)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid decision")
  end
  return '<!-- fkst:github-devloop:result:v1 proposal="' .. tostring(proposal_id)
    .. '" decision="' .. decision
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
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
    dedup_key = dedup_key({
      tostring(proposal.proposal_id),
      "comment",
      "thinking",
      tostring(proposal.dedup_key),
    }),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_result_label_request(repo, issue_number, reached)
  local to_state = reached.decision == "approve" and "ready" or "blocked"
  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    tostring(reached.proposal_id) .. "/label/" .. tostring(reached.decision),
    reached.source_ref
  )
end

function M.build_result_comment_request(repo, issue_number, reached)
  local marker = M.result_marker(reached.proposal_id, reached.decision, reached.dedup_key)
  local state = reached.decision == "approve" and "ready" or "blocked"
  local state_marker = M.state_marker(reached.proposal_id, state, reached.dedup_key)
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  local body = "github-devloop decision: " .. tostring(reached.decision)
    .. "\n\n" .. body_text
    .. "\n\n" .. state_marker
    .. "\n" .. marker
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = body,
    -- Include the consensus dedup_key (version) so a new decision/version writes a fresh result
    -- marker instead of being suppressed by an older same-direction github-proxy comment marker.
    dedup_key = tostring(reached.proposal_id) .. "/comment/" .. tostring(reached.decision)
      .. "/" .. (tostring(reached.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(reached.source_ref),
  }
end

function M.build_loop_comment_request(repo, issue_number, unresolved, n)
  local marker = M.loop_marker(unresolved.proposal_id, n, unresolved.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop no-consensus loop: " .. tostring(n) .. "\n\n" .. marker,
    dedup_key = tostring(unresolved.proposal_id) .. "/comment/loop/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.build_stuck_label_request(repo, issue_number, unresolved, n)
  return M.build_state_label_request(
    repo,
    issue_number,
    "stuck",
    tostring(unresolved.proposal_id) .. "/label/stuck/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    unresolved.source_ref
  )
end

function M.build_stuck_comment_request(repo, issue_number, unresolved, n)
  local marker = M.stuck_marker(unresolved.proposal_id, n, unresolved.dedup_key)
  local state_marker = M.state_marker(unresolved.proposal_id, "stuck", unresolved.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop stuck: no consensus after " .. tostring(n) .. " attempts\n\n"
      .. state_marker .. "\n" .. marker,
    dedup_key = tostring(unresolved.proposal_id) .. "/comment/stuck/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.build_meta_label_request(repo, issue_number, stuck, action)
  local to_state = action == "implement" and "ready" or "blocked"

  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    -- stuck.dedup_key already encodes proposal_id + version; do NOT also prefix proposal_id (that
    -- double-counts it and can push the meta dedup over max_dedup_len). The version-bearing
    -- stuck.dedup_key alone keeps it unique across attempts.
    dedup_key({
      "meta",
      "label",
      tostring(action),
      tostring(stuck.dedup_key),
    }),
    stuck.source_ref
  )
end

function M.build_meta_comment_request(repo, issue_number, stuck, action, reason)
  local marker = M.meta_marker(stuck.proposal_id, stuck.dedup_key)
  local to_state = action == "implement" and "ready" or "blocked"
  local state_marker = M.state_marker(stuck.proposal_id, to_state, stuck.dedup_key)
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local heading = "github-devloop meta action: " .. tostring(action)
  if action == "split" then
    heading = "github-devloop meta action: split\n\nSuggested split:\n" .. safe_reason
  else
    heading = heading .. "\n\nReason:\n" .. safe_reason
  end

  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = heading .. "\n\n" .. state_marker .. "\n" .. marker,
    -- The result comment is the durable meta fact. Key it by stuck version only so replayed
    -- non-deterministic meta runs cannot append contradictory same-version state markers.
    dedup_key = dedup_key({
      "meta",
      "comment",
      tostring(stuck.dedup_key),
    }),
    source_ref = M.normalize_source_ref(stuck.source_ref),
  }
end

function M.build_implementing_label_request(repo, issue_number, ready)
  return M.build_state_label_request(
    repo,
    issue_number,
    "implementing",
    dedup_key({
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
    dedup_key({
      "implement",
      "label",
      "impl-failed",
      tostring(reason or "failed"),
      tostring(ready.dedup_key),
    }),
    ready.source_ref
  )
end

function M.build_implementing_comment_request(repo, issue_number, ready, worktree)
  local marker = M.implementing_marker(ready.proposal_id, ready.dedup_key)
  local state_marker = M.state_marker(ready.proposal_id, "implementing", ready.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop implementation started"
      .. "\n\nWorktree: " .. tostring(worktree)
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = dedup_key({
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
  if #text > max_impl_output_len then
    text = text:sub(1, max_impl_output_len)
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
    dedup_key = dedup_key({
      "implement",
      "comment",
      "failure",
      safe_reason,
      tostring(ready.dedup_key),
    }),
    source_ref = M.normalize_source_ref(ready.source_ref),
  }
end

function M.is_supported_issue(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "issue"
    and payload.repo ~= nil
    and payload.number ~= nil
    and payload.title ~= nil
    and payload.updated_at ~= nil
    and M.issue_ref_round_trips(payload.repo, payload.number)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and (payload.decision == "approve" or payload.decision == "reject")
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and is_bounded_string(payload.body, max_body_len)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_unresolved(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_unresolved.v1"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and payload.body == nil
    and payload.angle_results == nil
    and payload.decision == nil
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_stuck(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.stuck.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.no_consensus_dedup_key)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.ready.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and has_bounded_source_ref(payload.source_ref)
end

return M
