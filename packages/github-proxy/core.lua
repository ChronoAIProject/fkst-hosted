local M = {}

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_WRITE = true,
}
local trusted_bot_login = nil
local max_branch_len = 160
local max_marker_value_len = 300
local state_stage_rank = {
  thinking = 100,
  ready = 500,
  implementing = 600,
  ["pr-open"] = 650,
  reviewing = 675,
  ["merge-ready"] = 690,
  merging = 695,
  fixing = 700,
  ["review-meta"] = 710,
  ["impl-failed"] = 750,
  blocked = 800,
  merged = 900,
}

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_git_ref_safe(value)
  if not is_bounded_string(value, max_branch_len) then
    return false
  end
  local text = tostring(value)
  if text:sub(1, 1) == "-" or text:sub(1, 1) == "/" then
    return false
  end
  if text:find("%.%.", 1, true) ~= nil
    or text:find("//", 1, true) ~= nil
    or text:find("@{", 1, true) ~= nil
    or text:sub(-1) == "/"
    or text:sub(-1) == "."
    or text:sub(-5) == ".lock" then
    return false
  end
  if text:find("[%s~^:?%[%]\\*]") ~= nil then
    return false
  end
  for segment in text:gmatch("[^/]+") do
    if segment == "." or segment == ".." or segment:sub(1, 1) == "." then
      return false
    end
  end
  return text:find("^[%w%._%-%/]+$") ~= nil
end

local function is_positive_number(value)
  local number = tonumber(value)
  return number ~= nil and number >= 1 and number % 1 == 0 and number <= 2147483647
end

local function is_safe_marker_value(value)
  return type(value) == "string" and value ~= "" and #value <= max_marker_value_len
end

local function is_git_sha(value)
  return type(value) == "string" and value:find("^[0-9A-Fa-f]+$") ~= nil and #value >= 6 and #value <= 64
end

function M.read_env_command(name)
  if not allowed_env[name] then
    error("env name is not allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

function M.read_env(name, exec)
  local run = exec or exec_sync
  if type(run) ~= "function" then
    error("read_env requires exec_sync")
  end
  local out = run(M.read_env_command(name))
  if out.exit_code ~= 0 then
    return nil
  end
  if out.stdout == "" then
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

  if trusted_bot_login == nil then
    error("github-proxy: FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
  end
  return trusted_bot_login
end

function M.entity_cache_key(repo, entity_type, number)
  return "github-proxy/" .. tostring(entity_type) .. "/" .. tostring(repo) .. "/" .. tostring(number)
end

function M.entity_dedup_key(repo, entity_type, number, updated_at)
  return tostring(repo)
    .. "#"
    .. tostring(entity_type)
    .. "#"
    .. tostring(number)
    .. "@"
    .. tostring(updated_at)
end

function M.issue_dedup_key(repo, number, updated_at)
  return M.entity_dedup_key(repo, "issue", number, updated_at)
end

-- Stable source pointer for the durable-delivery engine: a reliable consumer
-- re-derives the current entity from this ref (e.g. `gh issue view`) instead of
-- trusting a possibly-stale payload. ref is the entity identity WITHOUT the
-- version (updated_at lives in dedup_key / the payload).
function M.entity_source_ref(repo, entity_type, number)
  return {
    kind = "external",
    ref = tostring(repo) .. "#" .. tostring(entity_type) .. "/" .. tostring(number),
  }
end

local function sanitize_runtime_segment(value)
  local safe = tostring(value or ""):gsub("[^%w._-]", "-")
  safe = safe:gsub("-+", "-"):gsub("^-+", ""):gsub("-+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.issue_label_lock_key(repo, issue_number)
  local id = sanitize_runtime_segment(repo) .. "/issue/" .. sanitize_runtime_segment(issue_number)
  if #id > 180 then
    id = id:sub(1, 180)
  end
  return "github-proxy/label-lock/" .. id
end

function M.comment_marker(dedup_key)
  return "<!-- fkst:github-proxy:comment:" .. tostring(dedup_key) .. " -->"
end

function M.has_marker(comments_text, dedup_key)
  if comments_text == nil or comments_text == "" then
    return false
  end
  return tostring(comments_text):find(M.comment_marker(dedup_key), 1, true) ~= nil
end

local function comment_body(comment)
  if type(comment) == "table" then
    return tostring(comment.body or "")
  end
  return tostring(comment or "")
end

local function comment_author_login(comment)
  if type(comment) == "table" then
    if comment.author_login ~= nil then
      return tostring(comment.author_login)
    end
    if type(comment.author) == "table" and comment.author.login ~= nil then
      return tostring(comment.author.login)
    end
  end
  return nil
end

function M.parse_issue_comments(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
  local comments = {}
  for _, comment in ipairs(decoded.comments or {}) do
    table.insert(comments, {
      body = comment_body(comment),
      author_login = comment_author_login(comment),
    })
  end
  return comments
end

function M.has_trusted_marker(comments, dedup_key, bot_login)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.comment_marker(dedup_key)
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login and comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_trusted_comment_fragment(comments, fragment, bot_login)
  if type(comments) ~= "table" or type(fragment) ~= "string" or fragment == "" then
    return false
  end
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login and comment_body(comment):find(fragment, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.is_safe_branch(branch)
  return is_git_ref_safe(branch)
end

function M.is_safe_pr_number(pr_number)
  return is_positive_number(pr_number)
end

function M.is_safe_head_sha(head_sha)
  return is_git_sha(head_sha)
end

-- Decodes gh --json output via the engine-provided json.decode; requires a json-capable substrate runtime.
function M.parse_entity_list(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "[]")
  local entities = {}
  for _, item in ipairs(decoded) do
    -- Array-tagged so an empty labels list serializes as JSON [] (not {}) when
    -- the event payload leaves the engine via raise. See SPEC: a bare Lua {} is
    -- ambiguous and serializes as a JSON object; json.decode("[]") preserves [].
    local labels = json.decode("[]")
    for _, label in ipairs(item.labels or {}) do
      if type(label) == "table" and label.name ~= nil then
        table.insert(labels, tostring(label.name))
      elseif type(label) == "string" then
        table.insert(labels, label)
      end
    end
    table.insert(entities, {
      number = item.number,
      title = item.title,
      url = item.url,
      updated_at = item.updatedAt or item.updated_at,
      state = item.state,
      labels = labels,
    })
  end
  return entities
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

function M.parse_issue_state(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
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
    comments = M.parse_issue_comments(gh_json_stdout),
  }
end

local function trusted_comments(comments, bot_login)
  local result = {}
  if type(comments) ~= "table" then
    return result
  end
  for _, comment in ipairs(comments) do
    if comment_author_login(comment) == bot_login then
      table.insert(result, comment)
    end
  end
  return result
end

local function version_updated_at(version)
  local updated_at = ""
  for found in tostring(version or ""):gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    updated_at = found:gsub(":", "-")
  end
  return updated_at
end

local function version_loop_round(version)
  local n = tostring(version or ""):match("/loop/(%d+)$")
  return tonumber(n) or 0
end

local function version_fix_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("/fix/(%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

local function version_review_meta_action_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("/review%-meta%-action/(%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

local function version_order_key(version)
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

local function version_primary_key(version)
  local updated_at = version_updated_at(version)
  if updated_at ~= "" then
    return updated_at
  end
  return version_order_key(version)
end

local function version_sort_key(version, stage_rank)
  return {
    primary = version_primary_key(version),
    loop_n = version_loop_round(version),
    fix_n = version_fix_round(version),
    review_meta_action_n = version_review_meta_action_round(version),
    stage_rank = tonumber(stage_rank) or 0,
  }
end

local function marker_stage_rank(marker, state)
  return tonumber(marker:match('stage_rank="(%d+)"')) or state_stage_rank[state] or 0
end

local function compare_state_marker(current, candidate)
  if current == nil then
    return true
  end
  local current_key = version_sort_key(current.version, current.stage_rank)
  local candidate_key = version_sort_key(candidate.version, candidate.stage_rank)
  if candidate_key.primary ~= current_key.primary then
    return candidate_key.primary > current_key.primary
  end
  if candidate_key.loop_n ~= current_key.loop_n then
    return candidate_key.loop_n > current_key.loop_n
  end
  if candidate_key.fix_n ~= current_key.fix_n then
    return candidate_key.fix_n > current_key.fix_n
  end
  if candidate_key.review_meta_action_n ~= current_key.review_meta_action_n then
    return candidate_key.review_meta_action_n > current_key.review_meta_action_n
  end
  if candidate.version == current.version
    and ((current.state == "ready" and candidate.state == "blocked") or (current.state == "blocked" and candidate.state == "ready")) then
    return candidate.state == "blocked"
  end
  if candidate_key.stage_rank ~= current_key.stage_rank then
    return candidate_key.stage_rank > current_key.stage_rank
  end
  return false
end

function M.current_devloop_state(comments, proposal_id, bot_login)
  if not is_safe_marker_value(proposal_id) then
    return { state = nil, version = nil, stage_rank = 0 }
  end
  local current = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(trusted_comments(comments, bot_login)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_state = marker:match('state="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      if marker_proposal == proposal_id and state_stage_rank[marker_state] ~= nil then
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
  return current or { state = nil, version = nil, stage_rank = 0 }
end

function M.devloop_implementing_fact(comments, proposal_id, impl_version, bot_login)
  if not is_safe_marker_value(proposal_id) or not is_safe_marker_value(impl_version) then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:implementing:v1.-%-%->"
  for _, comment in ipairs(trusted_comments(comments, bot_login)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      local marker_base_branch = marker:match('base_branch="([^"]+)"')
      local marker_base_sha = marker:match('base_sha="([^"]+)"')
      if marker_proposal == proposal_id
        and marker_dedup == tostring(impl_version)
        and is_git_ref_safe(marker_branch)
        and is_git_sha(marker_head_sha)
        and is_git_ref_safe(marker_base_branch)
        and (marker_base_sha == nil or is_git_sha(marker_base_sha)) then
        return {
          proposal_id = marker_proposal,
          impl_version = marker_dedup,
          branch = marker_branch,
          head_sha = marker_head_sha,
          base_branch = marker_base_branch,
          base_sha = marker_base_sha,
        }
      end
    end
  end
  return nil
end

function M.has_devloop_pr_open_marker(comments, proposal_id, impl_version, bot_login)
  if not is_safe_marker_value(proposal_id) or not is_safe_marker_value(impl_version) then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(trusted_comments(comments, bot_login)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == proposal_id
        and marker:match('state="([^"]+)"') == "pr-open"
        and marker:match('version="([^"]*)"') == tostring(impl_version) then
        return true
      end
    end
  end
  return false
end

function M.parse_issue_list(gh_json_stdout)
  return M.parse_entity_list(gh_json_stdout)
end

function M.gh_issue_list_cmd(repo)
  return "gh issue list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
end

function M.gh_pr_list_cmd(repo)
  return "gh pr list --repo " .. shell_single_quote(repo) .. " --state all --json number,title,updatedAt,url,state,labels"
end

function M.gh_pr_list_head_cmd(repo, branch, base_branch)
  if not is_git_ref_safe(branch) then
    error("github-proxy: invalid branch")
  end
  if base_branch ~= nil and not is_git_ref_safe(base_branch) then
    error("github-proxy: invalid base branch")
  end
  local base_arg = ""
  if base_branch ~= nil then
    base_arg = " --base " .. shell_single_quote(base_branch)
  end
  return "gh pr list --repo " .. shell_single_quote(repo)
    .. " --head " .. shell_single_quote(branch)
    .. base_arg
    .. " --state open --json number,url,headRefName,baseRefName,state"
end

function M.gh_issue_view_pr_open_guard_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.parse_pr_list_for_head(gh_json_stdout, branch)
  local decoded = json.decode(gh_json_stdout or "[]")
  for _, item in ipairs(decoded) do
    local number = item.number
    local head = item.headRefName or item.head_ref_name
    local base = item.baseRefName or item.base_ref_name
    local state = tostring(item.state or "")
    if is_positive_number(number)
      and tostring(head or "") == tostring(branch)
      and state:lower() == "open" then
      return {
        number = tonumber(number),
        url = item.url,
        head_ref_name = head,
        base_ref_name = base,
        state = item.state,
      }
    end
  end
  return nil
end

function M.git_push_branch_cmd(branch)
  if not is_git_ref_safe(branch) then
    error("github-proxy: invalid branch")
  end
  return "git push -u origin " .. shell_single_quote(branch)
end

function M.git_show_ref_branch_cmd(branch)
  if not is_git_ref_safe(branch) then
    error("github-proxy: invalid branch")
  end
  return "git show-ref --verify refs/heads/" .. shell_single_quote(branch)
end

function M.parse_git_show_ref_head(stdout, branch)
  local head_sha, ref = tostring(stdout or ""):match("^%s*([0-9a-fA-F]+)%s+(%S+)")
  if is_git_sha(head_sha) and ref == "refs/heads/" .. tostring(branch) then
    return head_sha:lower()
  end
  return nil
end

function M.gh_pr_create_cmd(repo, branch, base_branch, title, body_file)
  if not is_git_ref_safe(branch) then
    error("github-proxy: invalid branch")
  end
  if base_branch ~= nil and not is_git_ref_safe(base_branch) then
    error("github-proxy: invalid base branch")
  end
  local base_arg = ""
  if base_branch ~= nil then
    base_arg = " --base " .. shell_single_quote(base_branch)
  end
  return "gh pr create --repo " .. shell_single_quote(repo)
    .. " --head " .. shell_single_quote(branch)
    .. base_arg
    .. " --title " .. shell_single_quote(title)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.parse_pr_create(stdout)
  local url = tostring(stdout or ""):match("(https?://%S+/pull/(%d+))")
  local number = url and url:match("/pull/(%d+)")
  if is_positive_number(number) then
    return {
      number = tonumber(number),
      url = url,
    }
  end
  return nil
end

function M.gh_pr_comment_cmd(repo, pr_number, body_file)
  return "gh pr comment " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_pr_view_head_oid_cmd(repo, pr_number)
  if not is_positive_number(pr_number) then
    error("github-proxy: invalid PR number")
  end
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json headRefOid,baseRefName,state,headRepository,headRepositoryOwner,isCrossRepository"
end

local function repository_name_with_owner(head_repository, head_repository_owner)
  if type(head_repository) == "string" then
    return head_repository
  end
  if type(head_repository) ~= "table" then
    return nil
  end
  if head_repository.nameWithOwner ~= nil and head_repository.nameWithOwner ~= "" then
    return tostring(head_repository.nameWithOwner)
  end
  if head_repository.name_with_owner ~= nil and head_repository.name_with_owner ~= "" then
    return tostring(head_repository.name_with_owner)
  end
  local name = head_repository.name
  local owner = nil
  if type(head_repository.owner) == "table" and head_repository.owner.login ~= nil then
    owner = head_repository.owner.login
  elseif type(head_repository_owner) == "table" and head_repository_owner.login ~= nil then
    owner = head_repository_owner.login
  elseif type(head_repository_owner) == "string" then
    owner = head_repository_owner
  end
  if owner ~= nil and name ~= nil then
    return tostring(owner) .. "/" .. tostring(name)
  end
  return nil
end

function M.parse_pr_view_head_state(gh_json_stdout, target_repo)
  local decoded = json.decode(gh_json_stdout or "{}")
  local head = decoded.headRefOid or decoded.head_ref_oid
  local state = decoded.state
  local head_repo = repository_name_with_owner(
    decoded.headRepository or decoded.head_repository,
    decoded.headRepositoryOwner or decoded.head_repository_owner
  )
  local is_cross_repository = decoded.isCrossRepository
  if is_cross_repository == nil then
    is_cross_repository = decoded.is_cross_repository
  end
  if is_git_sha(head) and state ~= nil then
    return {
      head_ref_oid = tostring(head):lower(),
      base_ref_name = decoded.baseRefName or decoded.base_ref_name,
      state = tostring(state),
      head_repository = head_repo,
      is_cross_repository = is_cross_repository,
      is_target_repository = target_repo ~= nil
        and head_repo ~= nil
        and tostring(head_repo):lower() == tostring(target_repo):lower(),
    }
  end
  return nil
end

function M.gh_pr_view_comments_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_view_comments_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json comments"
end

function M.gh_issue_view_labels_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels"
end

function M.parse_issue_labels(gh_json_stdout)
  local decoded = json.decode(gh_json_stdout or "{}")
  local labels = {}
  for _, label in ipairs(decoded.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end
  return labels
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_issue_edit_labels_cmd(repo, issue_number, add_labels, remove_labels)
  local cmd = "gh issue edit " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
  for _, label in ipairs(add_labels or {}) do
    cmd = cmd .. " --add-label " .. shell_single_quote(label)
  end
  for _, label in ipairs(remove_labels or {}) do
    cmd = cmd .. " --remove-label " .. shell_single_quote(label)
  end
  return cmd
end

return M
