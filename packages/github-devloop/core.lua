local M = {}

local max_key_len = 200
local max_dedup_len = 512
local max_title_len = 240
local max_body_len = 12000
local max_comments_len = 12000
local max_meta_reason_len = 2000
local max_impl_output_len = 2000
local max_pr_diff_len = 8000
local max_pr_issue_context_len = 3000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50
local max_version_key_len = 40
local max_worktree_prefix_len = 90
local max_branch_len = 160
local max_sha_len = 64
local max_pr_title_len = 240
local action_label = "⟦FKST:ACTION⟧"
local reason_label = "⟦FKST:REASON⟧"
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local untrusted_issue_data_begin = "BEGIN UNTRUSTED ISSUE DATA"
local untrusted_issue_data_end = "END UNTRUSTED ISSUE DATA"
local test_bot_login = "fkst-test-bot"

local enabled_label = "fkst-dev:enabled"
local thinking_label = "fkst-dev:thinking"
local ready_label = "fkst-dev:ready"
local implementing_label = "fkst-dev:implementing"
local pr_authorized_label = "fkst-dev:pr-authorized"
local pr_open_label = "fkst-dev:pr-open"
local reviewing_label = "fkst-dev:reviewing"
local merge_ready_label = "fkst-dev:merge-ready"
local merge_authorized_label = "fkst-dev:merge-authorized"
local merging_label = "fkst-dev:merging"
local merged_label = "fkst-dev:merged"
local fixing_label = "fkst-dev:fixing"
local review_meta_label = "fkst-dev:review-meta"
local fix_authorized_label = "fkst-dev:fix-authorized"
local impl_failed_label = "fkst-dev:impl-failed"
local blocked_label = "fkst-dev:blocked"
local stuck_label = "fkst-dev:stuck"
local loop_budget = 3

local state_labels = {
  [thinking_label] = true,
  [ready_label] = true,
  [implementing_label] = true,
  [pr_open_label] = true,
  [reviewing_label] = true,
  [merge_ready_label] = true,
  [merging_label] = true,
  [merged_label] = true,
  [fixing_label] = true,
  [review_meta_label] = true,
  [impl_failed_label] = true,
  [blocked_label] = true,
  [stuck_label] = true,
}

local label_by_state = {
  thinking = thinking_label,
  ready = ready_label,
  implementing = implementing_label,
  ["pr-open"] = pr_open_label,
  reviewing = reviewing_label,
  ["merge-ready"] = merge_ready_label,
  merging = merging_label,
  merged = merged_label,
  fixing = fixing_label,
  ["review-meta"] = review_meta_label,
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
  implementing = { "pr-open", "impl-failed" },
  ["pr-open"] = { "reviewing" },
  reviewing = { "merge-ready", "fixing", "review-meta" },
  ["merge-ready"] = { "merging", "fixing", "blocked" },
  merging = { "merged", "fixing", "blocked" },
  merged = {},
  fixing = { "reviewing", "review-meta" },
  ["review-meta"] = { "fixing", "merge-ready", "blocked" },
  ["impl-failed"] = {},
  blocked = {},
}

local state_order = { "thinking", "ready", "implementing", "pr-open", "reviewing", "merge-ready", "fixing", "impl-failed", "blocked", "stuck", "review-meta", "merging", "merged" }
local state_stage_rank = {
  thinking = 100,
  stuck = 300,
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

local function decimal_checksum(value)
  local hash = 2166136261
  local text = tostring(value or "")
  for i = 1, #text do
    hash = (hash * 16777619 + text:byte(i)) % 4294967291
  end
  return string.format("%010d", hash)
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function has_value(values, expected)
  if type(values) ~= "table" then
    return false
  end
  for _, value in ipairs(values) do
    if value == expected then
      return true
    end
  end
  return false
end

local function is_meta_action(value)
  return value == "implement" or value == "split" or value == "block"
end

local function is_review_meta_action(value)
  return value == "fix" or value == "accept" or value == "block"
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

local function is_git_sha(value)
  return is_bounded_string(value, max_sha_len) and tostring(value):find("^%x+$") ~= nil
end

local function is_positive_pr_number(value)
  local number = tonumber(value)
  return number ~= nil and number >= 1 and number % 1 == 0 and number <= 2147483647
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

function M.safe_version_segment(version)
  local safe = M.sanitize_key(version, false):gsub("[/#]", "-"):gsub("%-+", "-")
  safe = safe:gsub("^%-+", ""):gsub("%-+$", "")
  if safe == "" then
    safe = "version"
  end
  if #safe > max_version_key_len then
    local suffix = "-" .. decimal_checksum(version)
    safe = safe:sub(1, max_version_key_len - #suffix):gsub("%-+$", "") .. suffix
  end
  if safe == "" then
    return "version"
  end
  return safe
end

function M.safe_pr_review_repo_segment(repo)
  local safe = M.safe_repo(repo):gsub("/", "-"):gsub("%-+", "-")
  safe = safe:gsub("^%-+", ""):gsub("%-+$", "")
  if safe == "" then
    safe = "repo"
  end
  local suffix = "-" .. decimal_checksum(repo)
  local limit = 48
  if #safe > limit or safe:sub(-#suffix) ~= suffix then
    safe = safe:sub(1, limit - #suffix):gsub("%-+$", "") .. suffix
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

function M.safe_head_segment(head_sha)
  if not is_git_sha(head_sha) then
    error("github-devloop: invalid head sha")
  end
  return tostring(head_sha)
end

function M.pr_review_proposal_id(repo, pr_number, version, head_sha)
  if not is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pr number")
  end
  if head_sha == nil then
    error("github-devloop: missing reviewed head sha")
  end
  return "github-devloop/pr-review/"
    .. M.safe_pr_review_repo_segment(repo)
    .. "/"
    .. M.safe_issue(pr_number)
    .. "/"
    .. M.safe_version_segment(version)
    .. "/"
    .. M.safe_head_segment(head_sha)
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

function M.parse_pr_review_proposal_id(id)
  if type(id) ~= "string" then
    return nil
  end

  local rest = id:match("^github%-devloop/pr%-review/(.+)$")
  if rest == nil then
    return nil
  end

  local head_sha = rest:match("/([^/]+)$")
  local without_head = head_sha and rest:sub(1, #rest - #head_sha - 1) or nil
  local version = without_head and without_head:match("/([^/]+)$") or nil
  local without_version = version and without_head:sub(1, #without_head - #version - 1) or nil
  local pr_number = without_version and without_version:match("/([^/]+)$") or nil
  local repo = pr_number and without_version:sub(1, #without_version - #pr_number - 1) or nil
  if repo == nil or repo == "" or pr_number == nil or pr_number == "" or version == nil or version == "" or head_sha == nil or head_sha == "" then
    return nil
  end
  if not is_positive_pr_number(pr_number) then
    return nil
  end
  if not is_git_sha(head_sha) then
    return nil
  end
  if not is_path_safe_key(repo, 64)
    or M.safe_issue(pr_number) ~= pr_number
    or M.safe_version_segment(version) ~= version
    or M.safe_head_segment(head_sha) ~= head_sha then
    return nil
  end
  return repo, pr_number, version, head_sha
end

function M.parse_pr_source_ref(source_ref)
  if type(source_ref) ~= "table" or source_ref.kind ~= "external" then
    return nil
  end
  local ref = tostring(source_ref.ref or "")
  local pr_number = ref:match("#pr/(%d+)$")
  local repo = pr_number and ref:sub(1, #ref - #("#pr/" .. pr_number)) or nil
  if repo == nil or repo == "" or not is_positive_pr_number(pr_number) then
    return nil
  end
  if M.safe_repo(repo) == "" then
    return nil
  end
  return repo, pr_number
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

function M.is_safe_pr_review_result_ref(proposal_id, dedup_key)
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

  local repo, pr_number = M.parse_pr_review_proposal_id(proposal_id)
  return repo ~= nil and pr_number ~= nil
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

function M.review_result_lock_key(issue_proposal_id)
  return M.transition_lock_key(issue_proposal_id)
end

function M.review_lock_key(proposal_id)
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

function M.implement_branch(repo, issue_number, impl_version)
  local safe_repo = M.safe_repo(repo)
  local safe_issue = M.safe_issue(issue_number)
  local safe_version = M.sanitize_key(impl_version, false):gsub("[/#]", "-"):gsub("%-+", "-")
  safe_version = safe_version:gsub("^%-+", ""):gsub("%-+$", ""):gsub("%.+$", "")
  if safe_version == "" then
    safe_version = "version"
  end

  local prefix = "devloop/issue/" .. safe_repo .. "/" .. safe_issue .. "/"
  local suffix = "-" .. decimal_checksum(tostring(repo) .. "#" .. tostring(issue_number) .. "#" .. tostring(impl_version))
  local version_limit = max_branch_len - #prefix - #suffix
  if version_limit < 12 then
    version_limit = 12
  end
  if #safe_version > version_limit then
    safe_version = safe_version:sub(1, version_limit):gsub("%-+$", ""):gsub("%.+$", "")
  end
  if safe_version == "" then
    safe_version = "version"
  end

  local branch = prefix .. safe_version .. suffix
  if not is_git_ref_safe(branch) or #branch > max_branch_len then
    error("github-devloop: invalid deterministic implementation branch")
  end
  return branch
end

function M.implement_worktree_path(runtime_root, repo, issue_number, impl_version)
  local root = trim(runtime_root)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: invalid FKST_RUNTIME_ROOT")
  end
  local slug = M.safe_issue_slug(repo, issue_number)
  local suffix = decimal_checksum(tostring(repo) .. "#" .. tostring(issue_number) .. "#" .. tostring(impl_version))
  return root:gsub("/+$", "") .. "/worktrees/devloop-" .. slug .. "-" .. suffix
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

function M.bounded_pr_diff(value)
  local text = tostring(value or "")
  if text == "" then
    return "(empty PR diff)"
  end
  if #text <= max_pr_diff_len then
    return text
  end
  return text:sub(1, max_pr_diff_len)
end

function M.max_body_len()
  return max_body_len
end

function M.max_pr_diff_len()
  return max_pr_diff_len
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
    local sentinel_line = line:match("^%s*[+%- ]?%s*(.+)$") or line
    if sentinel_line:match("^%s*" .. action_label) ~= nil
      or sentinel_line:match("^%s*" .. reason_label) ~= nil
      or sentinel_line:match("^%s*" .. verdict_label) ~= nil
      or sentinel_line:match("^%s*" .. reply_label) ~= nil
      or trim(line) == untrusted_issue_data_begin
      or trim(line) == untrusted_issue_data_end
      or trim(sentinel_line) == untrusted_issue_data_begin
      or trim(sentinel_line) == untrusted_issue_data_end
      or line:find("<!%-%- fkst:") ~= nil
      or line:find("&lt;!%-%- fkst:") ~= nil then
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

function M.gh_issue_view_open_pr_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,labels,comments"
end

function M.gh_issue_view_reviewing_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.gh_issue_view_review_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_fix_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_review_loop_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_review_meta_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments"
end

function M.gh_issue_view_merge_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,labels,comments,state"
end

function M.gh_pr_view_origin_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,comments"
end

function M.gh_pr_view_fix_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,comments,headRepository,headRepositoryOwner,isCrossRepository"
end

function M.gh_pr_view_merge_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json headRefName,headRefOid,state,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup,latestReviews"
end

function M.gh_pr_merge_cmd(repo, pr_number, head_sha)
  if tostring(head_sha or "") == "" then
    error("github-devloop: invalid merge head sha")
  end
  return "gh pr merge " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --merge"
    .. " --match-head-commit " .. shell_single_quote(head_sha)
end

function M.gh_issue_comment_cmd(repo, issue_number, body_file)
  return "gh issue comment " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --body-file " .. shell_single_quote(body_file)
end

function M.gh_issue_close_cmd(repo, issue_number)
  return "gh issue close " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
end

function M.gh_pr_diff_cmd(repo, pr_number)
  return "gh pr diff " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
end

function M.gh_pr_view_head_cmd(repo, pr_number)
  return "gh pr view " .. shell_single_quote(pr_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json headRefName,state"
end

function M.git_status_cmd(worktree)
  return "git -C " .. shell_single_quote(worktree) .. " status --porcelain"
end

function M.git_add_all_cmd(worktree)
  return "git -C " .. shell_single_quote(worktree) .. " add -A"
end

function M.git_commit_cmd(worktree, message)
  local bounded_message = tostring(message or "")
  if bounded_message == "" or #bounded_message > 200 then
    error("github-devloop: invalid git commit message")
  end
  return "git -C " .. shell_single_quote(worktree) .. " commit -m " .. shell_single_quote(bounded_message)
end

function M.git_current_branch_cmd(worktree)
  return "git -C " .. shell_single_quote(worktree) .. " rev-parse --abbrev-ref HEAD"
end

function M.git_head_sha_cmd(worktree)
  return "git -C " .. shell_single_quote(worktree) .. " rev-parse HEAD"
end

function M.git_base_head_cmd()
  return "git rev-parse HEAD"
end

function M.git_show_ref_branch_cmd(branch)
  return "git show-ref --verify --quiet refs/heads/" .. shell_single_quote(branch)
end

function M.git_show_ref_cmd(worktree, branch)
  return "git -C " .. shell_single_quote(worktree) .. " show-ref --verify --quiet refs/heads/" .. shell_single_quote(branch)
end

function M.git_branch_ahead_count_cmd(base, branch)
  if not is_git_sha(base) then
    error("github-devloop: invalid base head")
  end
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git rev-list --count " .. shell_single_quote(base) .. "..refs/heads/" .. shell_single_quote(branch)
end

function M.git_branch_head_cmd(branch)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git rev-parse --verify refs/heads/" .. shell_single_quote(branch)
end

function M.git_push_branch_cmd(branch)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "git push origin " .. shell_single_quote(branch)
end

function M.read_runtime_root_cmd()
  return 'printf %s "$FKST_RUNTIME_ROOT"'
end

function M.git_worktree_add_new_branch_cmd(worktree, branch, base)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not is_git_sha(base) then
    error("github-devloop: invalid base head")
  end
  return "mkdir -p " .. shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add -b " .. shell_single_quote(branch)
    .. " " .. shell_single_quote(worktree)
    .. " " .. shell_single_quote(base)
end

function M.git_worktree_add_existing_branch_cmd(worktree, branch)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return "mkdir -p " .. shell_single_quote(tostring(worktree):gsub("/+$", ""):match("^(.*)/[^/]+$") or ".")
    .. " && git worktree add " .. shell_single_quote(worktree)
    .. " " .. shell_single_quote(branch)
end

function M.git_worktree_list_cmd()
  return "git worktree list --porcelain"
end

function M.find_worktree_for_branch(stdout, branch)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  local wanted = "refs/heads/" .. tostring(branch)
  local path = nil
  for line in (tostring(stdout or "") .. "\n"):gmatch("([^\n]*)\n") do
    if line == "" then
      path = nil
    else
      local current_path = line:match("^worktree%s+(.+)$")
      if current_path ~= nil then
        path = current_path
      elseif line == "branch " .. wanted and path ~= nil and path ~= "" then
        return path
      end
    end
  end
  return nil
end

function M.git_rev_parse_branch_cmd(worktree, branch)
  return "git -C " .. shell_single_quote(worktree) .. " rev-parse --verify refs/heads/" .. shell_single_quote(branch)
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
        created_at = comment.createdAt or comment.created_at,
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

function M.parse_issue_view_open_pr(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    labels = result.labels,
    comments = result.comments,
  }
end

function M.parse_issue_view_reviewing(stdout)
  return M.parse_issue_view_result(stdout)
end

function M.parse_issue_view_review(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_fix(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_loop(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_review_meta(stdout)
  return M.parse_issue_view_meta(stdout)
end

function M.parse_issue_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_meta(stdout)
  result.state = decoded.state
  return result
end

local function repository_name_with_owner(head_repository, head_repository_owner)
  if type(head_repository) == "string" then
    return head_repository
  end
  if type(head_repository) ~= "table" then
    return nil
  end
  if head_repository.nameWithOwner ~= nil then
    return tostring(head_repository.nameWithOwner)
  end
  if head_repository.name_with_owner ~= nil then
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

function M.parse_pr_view_origin(stdout)
  local decoded = json.decode(stdout or "{}")
  local head_repo = repository_name_with_owner(
    decoded.headRepository or decoded.head_repository,
    decoded.headRepositoryOwner or decoded.head_repository_owner
  )
  local is_cross_repository = decoded.isCrossRepository
  if is_cross_repository == nil then
    is_cross_repository = decoded.is_cross_repository
  end
  return {
    head_ref_name = decoded.headRefName or decoded.head_ref_name,
    head_sha = decoded.headRefOid or decoded.head_ref_oid,
    state = decoded.state,
    comments = M.comments_from_json(decoded.comments),
    head_repository = head_repo,
    is_cross_repository = is_cross_repository,
  }
end

function M.parse_pr_view_fix(stdout)
  return M.parse_pr_view_origin(stdout)
end

local function status_rollup_entries(value)
  if type(value) ~= "table" then
    return {}
  end
  if type(value.nodes) == "table" then
    return value.nodes
  end
  return value
end

local function review_entries(value)
  if type(value) ~= "table" then
    return {}
  end
  if type(value.nodes) == "table" then
    return value.nodes
  end
  return value
end

local function review_commit_id(review)
  if type(review) ~= "table" then
    return nil
  end
  local commit = review.commit_id or review.commitId or review.commitOID or review.commitOid or review.commit
  if type(commit) == "table" then
    commit = commit.oid or commit.id
  end
  if is_git_sha(commit) then
    return tostring(commit)
  end
  return nil
end

function M.parse_pr_view_merge(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_pr_view_origin(stdout)
  result.mergeable = decoded.mergeable
  result.merge_state_status = decoded.mergeStateStatus or decoded.merge_state_status
  result.status_check_rollup = status_rollup_entries(decoded.statusCheckRollup or decoded.status_check_rollup)
  result.merged_at = decoded.mergedAt or decoded.merged_at
  result.latest_reviews = review_entries(decoded.latestReviews or decoded.latest_reviews)
  return result
end

function M.parse_pr_view_head_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return {
    head_ref_name = decoded.headRefName or decoded.head_ref_name,
    state = decoded.state,
  }
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
    and state ~= "pr-open"
    and state ~= "reviewing"
    and state ~= "review-meta"
    and state ~= "merge-ready"
    and state ~= "merging"
    and state ~= "merged"
    and state ~= "fixing"
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

local function comment_created_at(comment)
  if type(comment) == "table" then
    return comment.created_at
  end
  return nil
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

function M.comment_created_at(comment)
  return comment_created_at(comment)
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
    "branch=" .. tostring(request and request.branch or ""),
    "pr=" .. tostring(request and request.pr_number or ""),
    "dedup_key=" .. tostring(request and request.dedup_key or ""),
  })
end

function M.log_raise(dept, proposal_id, queue, payload)
  if queue == "github-proxy.github_issue_label_request"
    or queue == "github-proxy.github_issue_comment_request"
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
  local n = tostring(version or ""):match("[/-]loop[/-](%d+)$")
  return tonumber(n) or 0
end

function M.version_fix_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]fix[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.version_review_meta_action_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]review%-meta%-action[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.version_review_loop_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]review%-loop[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.next_fix_version(version)
  local base = tostring(version or "")
  local next_n = M.version_fix_round(base) + 1
  return base .. "/fix/" .. tostring(next_n)
end

function M.fix_version_from_review_version(version)
  return M.next_fix_version(version)
end

function M.next_review_meta_action_version(version)
  local base = tostring(version or "")
  local next_n = M.version_review_meta_action_round(base) + 1
  return base .. "/review-meta-action/" .. tostring(next_n)
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
    fix_n = M.version_fix_round(version),
    review_loop_n = M.version_review_loop_round(version),
    review_meta_action_n = M.version_review_meta_action_round(version),
    stage_rank = tonumber(stage_rank) or 0,
  }
end

local function marker_stage_rank(marker, state)
  local explicit_rank = tonumber(marker:match('stage_rank="(%d+)"'))
  return explicit_rank or M.stage_rank(state)
end

local function compare_version_keys(left, right)
  if left.primary ~= right.primary then
    return left.primary > right.primary and 1 or -1
  end
  if left.loop_n ~= right.loop_n then
    return left.loop_n > right.loop_n and 1 or -1
  end
  if left.fix_n ~= right.fix_n then
    return left.fix_n > right.fix_n and 1 or -1
  end
  if left.review_meta_action_n ~= right.review_meta_action_n then
    return left.review_meta_action_n > right.review_meta_action_n and 1 or -1
  end
  if left.review_loop_n ~= right.review_loop_n then
    return left.review_loop_n > right.review_loop_n and 1 or -1
  end
  return 0
end

local function versions_equivalent(left, right)
  if left == nil or right == nil then
    return left == right
  end
  if tostring(left) == tostring(right) then
    return true
  end
  return M.safe_version_segment(left) == M.safe_version_segment(right)
end

local function strip_transition_version_suffixes(version)
  local text = tostring(version or "")
  local previous = nil
  while previous ~= text do
    previous = text
    text = text
      :gsub("/review%-meta%-action/%d+$", "")
      :gsub("%-review%-meta%-action%-%d+$", "")
      :gsub("/review%-loop/%d+$", "")
      :gsub("%-review%-loop%-%d+$", "")
      :gsub("/fix/%d+$", "")
      :gsub("%-fix%-%d+$", "")
  end
  return text
end

local function strip_latest_fix_version_suffix(version)
  return tostring(version or "")
    :gsub("/fix/%d+$", "")
    :gsub("%-fix%-%d+$", "")
end

local function compare_same_base_transition_versions(incoming_version, current_version)
  local incoming_key = version_sort_key(incoming_version, 0)
  local current_key = version_sort_key(current_version, 0)
  if incoming_key.loop_n ~= current_key.loop_n then
    return incoming_key.loop_n > current_key.loop_n and 1 or -1
  end
  if incoming_key.fix_n ~= current_key.fix_n then
    return incoming_key.fix_n > current_key.fix_n and 1 or -1
  end
  if incoming_key.review_meta_action_n ~= current_key.review_meta_action_n then
    return incoming_key.review_meta_action_n > current_key.review_meta_action_n and 1 or -1
  end
  if incoming_key.review_loop_n ~= current_key.review_loop_n then
    return incoming_key.review_loop_n > current_key.review_loop_n and 1 or -1
  end
  return 0
end

local function compare_transition_versions(incoming_version, current_version)
  if incoming_version == current_version then
    return 0
  end
  if incoming_version == nil then
    return current_version == nil and 0 or -1
  end
  if current_version == nil then
    return 1
  end
  local incoming_base = strip_transition_version_suffixes(incoming_version)
  local current_base = strip_transition_version_suffixes(current_version)
  if versions_equivalent(incoming_base, current_base) then
    return compare_same_base_transition_versions(incoming_version, current_version)
  end
  return compare_version_keys(version_sort_key(incoming_version, 0), version_sort_key(current_version, 0))
end

local function compare_state_marker(a, b)
  if a == nil then
    return true
  end
  local a_key = version_sort_key(a.version, a.stage_rank)
  local b_key = version_sort_key(b.version, b.stage_rank)
  local version_order = compare_version_keys(b_key, a_key)
  if version_order ~= 0 then
    return version_order > 0
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
    and compare_transition_versions(incoming_version, current.version) < 0 then
    return "stale"
  end
  local status = M.transition_status(current, from_states, to_state)
  return status
end

function M.cyclic_transition_status(current, from_states, to_state, incoming_version, target_version)
  local current_state = current
  local current_version = nil
  if type(current) == "table" then
    current_state = current.state
    current_version = current.version
  end
  if incoming_version == nil then
    return M.transition_status(current, from_states, to_state)
  end
  if target_version ~= nil and current_state == to_state and versions_equivalent(current_version, target_version) then
    return "idempotent"
  end

  local version_order = compare_transition_versions(incoming_version, current_version)
  if version_order > 0 then
    return "pending"
  end
  if version_order < 0 then
    return "stale"
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
  if M.stage_rank(to_state) > M.stage_rank(current_state) then
    return "apply"
  end
  return "stale"
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
      and compare_transition_versions(incoming_version, current.version) < 0 then
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
  if (to_state == "fixing" or to_state == "reviewing" or to_state == "merged")
    and not has_value(remove_labels, merge_authorized_label) then
    table.insert(remove_labels, merge_authorized_label)
  end
  return { add_label }, remove_labels
end

function M.state_label_hint_matches(labels, state)
  local expected_label = M.state_label(state)
  if expected_label == nil then
    return false
  end

  local has_expected = false
  for _, label in ipairs(labels or {}) do
    local label_text = tostring(label)
    if label_text == expected_label then
      has_expected = true
    elseif state_labels[label_text] then
      return false
    end
  end
  return has_expected
end

function M.build_reconcile_state_label_request(repo, issue_number, proposal_id, state, version, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    state,
    dedup_key({
      "reconcile",
      "label",
      tostring(proposal_id),
      tostring(state),
      tostring(version or "unversioned"),
    }),
    source_ref
  )
end

function M.has_terminal_label(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, pr_open_label)
    or M.has_label(labels, reviewing_label)
    or M.has_label(labels, review_meta_label)
    or M.has_label(labels, merge_ready_label)
    or M.has_label(labels, merging_label)
    or M.has_label(labels, merged_label)
    or M.has_label(labels, fixing_label)
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

function M.has_pr_authorized_label(labels)
  return M.has_label(labels, pr_authorized_label)
end

function M.has_pr_open_label(labels)
  return M.has_label(labels, pr_open_label)
end

function M.has_reviewing_label(labels)
  return M.has_label(labels, reviewing_label)
end

function M.has_merge_ready_label(labels)
  return M.has_label(labels, merge_ready_label)
end

function M.has_merge_authorized_label(labels)
  return M.has_label(labels, merge_authorized_label)
end

function M.has_merging_label(labels)
  return M.has_label(labels, merging_label)
end

function M.has_merged_label(labels)
  return M.has_label(labels, merged_label)
end

function M.has_fixing_label(labels)
  return M.has_label(labels, fixing_label)
end

function M.has_fix_authorized_label(labels)
  return M.has_label(labels, fix_authorized_label)
end

function M.has_review_meta_label(labels)
  return M.has_label(labels, review_meta_label)
end

function M.has_impl_failed_label(labels)
  return M.has_label(labels, impl_failed_label)
end

function M.has_decision_terminal_label(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, pr_open_label)
    or M.has_label(labels, reviewing_label)
    or M.has_label(labels, review_meta_label)
    or M.has_label(labels, merge_ready_label)
    or M.has_label(labels, merging_label)
    or M.has_label(labels, merged_label)
    or M.has_label(labels, fixing_label)
    or M.has_label(labels, impl_failed_label)
    or M.has_label(labels, blocked_label)
end

function M.is_loop_terminal(labels)
  return M.has_label(labels, ready_label)
    or M.has_label(labels, implementing_label)
    or M.has_label(labels, pr_open_label)
    or M.has_label(labels, reviewing_label)
    or M.has_label(labels, review_meta_label)
    or M.has_label(labels, merge_ready_label)
    or M.has_label(labels, merging_label)
    or M.has_label(labels, merged_label)
    or M.has_label(labels, fixing_label)
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

local function upper_text(value)
  return tostring(value or ""):upper()
end

local function check_entry_state(entry)
  if type(entry) ~= "table" then
    return nil, nil
  end
  return upper_text(entry.state or entry.status), upper_text(entry.conclusion)
end

local green_check_conclusions = {
  SUCCESS = true,
  NEUTRAL = true,
  SKIPPED = true,
}

local green_status_states = {
  SUCCESS = true,
}

local red_status_states = {
  ERROR = true,
  FAILURE = true,
}

function M.pr_rollup_green(pr)
  local entries = type(pr) == "table" and pr.status_check_rollup or nil
  if type(entries) ~= "table" or #entries == 0 then
    return false, "missing-status-rollup"
  end
  for _, entry in ipairs(entries) do
    local state, conclusion = check_entry_state(entry)
    if state == "COMPLETED" then
      if not green_check_conclusions[conclusion] then
        return false, "rollup-red"
      end
    elseif conclusion == "" and green_status_states[state] then
      -- Legacy StatusContext entries report state=SUCCESS without a conclusion.
    elseif conclusion == "" and red_status_states[state] then
      return false, "rollup-red"
    else
      return false, "rollup-pending"
    end
  end
  return true, "rollup-green"
end

function M.pr_mergeable(pr)
  if type(pr) ~= "table" then
    return false, "missing-pr"
  end
  local mergeable = upper_text(pr.mergeable)
  local merge_state = upper_text(pr.merge_state_status)
  if mergeable == "UNKNOWN" then
    return false, "mergeable-unknown"
  end
  if mergeable ~= "MERGEABLE" then
    if mergeable == "" then
      return false, "missing-mergeability"
    end
    return false, "mergeable-" .. mergeable:lower()
  end
  if merge_state ~= "CLEAN" then
    if merge_state == "" then
      return false, "missing-mergeability"
    end
    return false, "merge-state-" .. merge_state:lower()
  end
  return true, "mergeable"
end

function M.is_ci_red_reason(reason)
  return tostring(reason or "") == "rollup-red"
end

function M.is_not_mergeable_reason(reason)
  local text = tostring(reason or "")
  return text == "mergeable-conflicting"
    or text == "mergeable-false"
    or text == "merge-state-dirty"
    or text == "merge-state-conflicting"
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

function M.review_loop_marker(review_proposal_id, issue_proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:review-loop:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.review_meta_trigger_marker(review_proposal_id, issue_proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:review-meta-trigger:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.review_meta_marker(issue_proposal_id, dedup_key, action, version)
  local fields = ""
  if action ~= nil then
    if not is_review_meta_action(action) then
      error("github-devloop: invalid review-meta action")
    end
    fields = fields .. '" action="' .. tostring(action)
  end
  if version ~= nil then
    fields = fields .. '" version="' .. tostring(version)
  end
  return '<!-- fkst:github-devloop:review-meta:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. fields
    .. '" -->'
end

function M.fix_marker(issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  if not is_git_sha(old_head_sha) or not is_git_sha(new_head_sha) then
    error("github-devloop: invalid fix head sha")
  end
  return '<!-- fkst:github-devloop:fix:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" old_head_sha="' .. tostring(old_head_sha)
    .. '" new_head_sha="' .. tostring(new_head_sha)
    .. '" -->'
end

function M.merge_gate_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha, reason)
  if not is_positive_pr_number(pr_number) or not is_git_sha(head_sha) then
    error("github-devloop: invalid merge-gate marker")
  end
  return '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" reason="' .. tostring(M.sanitize_key(reason or "gate-failed", false):gsub("/", "-"))
    .. '" -->'
end

function M.implementing_marker(proposal_id, dedup_key, branch, head_sha)
  local fields = ""
  if branch ~= nil then
    fields = fields .. '" branch="' .. tostring(branch)
  end
  if head_sha ~= nil then
    fields = fields .. '" head_sha="' .. tostring(head_sha)
  end
  return '<!-- fkst:github-devloop:implementing:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. fields
    .. '" -->'
end

function M.pr_link_marker(proposal_id, pr_number, branch, impl_version)
  if not is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pr number")
  end
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" -->'
end

function M.pr_link_marker_template(proposal_id, branch, impl_version)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="{{pr_number}}"'
    .. ' branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" -->'
end

function M.pr_origin_marker(proposal_id, issue_number, branch, impl_version)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  return '<!-- fkst:github-devloop:pr-origin:v1 proposal="' .. tostring(proposal_id)
    .. '" issue="' .. tostring(issue_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" -->'
end

function M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid review decision")
  end
  return '<!-- fkst:github-devloop:review-result:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.merge_ready_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha)
  if not is_positive_pr_number(pr_number) then
    error("github-devloop: invalid merge-ready pr number")
  end
  if not is_git_sha(head_sha) then
    error("github-devloop: invalid merge-ready head sha")
  end
  if not is_bounded_string(version, max_dedup_len)
    or not is_bounded_string(review_proposal_id, max_key_len)
    or not is_bounded_string(review_dedup_key, max_dedup_len) then
    error("github-devloop: invalid merge-ready marker")
  end
  return '<!-- fkst:github-devloop:merge-ready:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.merged_marker(issue_proposal_id, pr_number, version, head_sha)
  if not is_positive_pr_number(pr_number) or not is_git_sha(head_sha) then
    error("github-devloop: invalid merged marker")
  end
  return '<!-- fkst:github-devloop:merged:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.merging_marker(issue_proposal_id, pr_number, version, head_sha)
  if not is_positive_pr_number(pr_number) or not is_git_sha(head_sha) then
    error("github-devloop: invalid merging marker")
  end
  return '<!-- fkst:github-devloop:merging:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.review_reject_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local review_proposal = marker:match('proposal="([^"]+)"')
      local marker_issue = marker:match('issue_proposal="([^"]+)"')
      local decision = marker:match('decision="([^"]+)"')
      local review_dedup = marker:match('dedup="([^"]*)"')
      local _, _, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
      if marker_issue == tostring(issue_proposal_id)
        and decision == "reject"
        and review_version == M.safe_version_segment(strip_latest_fix_version_suffix(issue_version))
        and is_bounded_string(review_dedup, max_dedup_len)
        and is_git_sha(reviewed_head_sha) then
        return {
          review_proposal_id = review_proposal,
          review_dedup_key = review_dedup,
          reviewed_head_sha = reviewed_head_sha,
          review_reason = comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.review_meta_fix_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-meta:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local action = marker:match('action="([^"]+)"')
      local version = marker:match('version="([^"]*)"')
      if marker_issue == tostring(issue_proposal_id)
        and marker_dedup ~= nil
        and action == "fix"
        and version == tostring(issue_version) then
        return {
          review_dedup_key = marker_dedup,
          review_reason = comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.merge_gate_fix_fact(comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merge%-gate:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_review_proposal = marker:match('review_proposal="([^"]+)"')
      local marker_review_dedup = marker:match('review_dedup="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and marker_version == tostring(issue_version)
        and is_bounded_string(marker_review_proposal, max_key_len)
        and is_bounded_string(marker_review_dedup, max_dedup_len)
        and is_git_sha(marker_head_sha) then
        return {
          review_proposal_id = marker_review_proposal,
          review_dedup_key = marker_review_dedup,
          reviewed_head_sha = marker_head_sha,
          review_reason = comment_body(comment),
        }
      end
    end
  end
  return nil
end

function M.merge_ready_fact(comments, issue_proposal_id, issue_version, pr_number)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merge%-ready:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_review_proposal = marker:match('review_proposal="([^"]+)"')
      local marker_review_dedup = marker:match('review_dedup="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and (pr_number == nil or tostring(marker_pr) == tostring(pr_number))
        and tostring(marker_version) == tostring(issue_version)
        and is_bounded_string(marker_review_proposal, max_key_len)
        and is_bounded_string(marker_review_dedup, max_dedup_len)
        and is_git_sha(marker_head_sha) then
        return {
          proposal_id = marker_issue,
          pr_number = tonumber(marker_pr),
          version = marker_version,
          review_proposal_id = marker_review_proposal,
          review_dedup_key = marker_review_dedup,
          head_sha = marker_head_sha,
          comment_created_at = comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.merge_authorization_matches_fact(fact, pr)
  if type(fact) ~= "table" or tostring(fact.head_sha or "") == "" then
    return false
  end
  if type(pr) ~= "table" then
    return false
  end
  if tostring(pr.head_sha or "") ~= tostring(fact.head_sha) then
    return false
  end
  local approved_at_head = false
  for _, review in ipairs(pr.latest_reviews or {}) do
    local state = upper_text(review.state)
    if state == "CHANGES_REQUESTED" then
      return false
    end
    if state == "APPROVED" and tostring(review_commit_id(review) or "") == tostring(fact.head_sha) then
      approved_at_head = true
    end
  end
  return approved_at_head
end

function M.merging_fact(comments, issue_proposal_id, pr_number, version, head_sha)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:merging:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_issue == tostring(issue_proposal_id)
        and tostring(marker_pr) == tostring(pr_number)
        and tostring(marker_version) == tostring(version)
        and tostring(marker_head_sha) == tostring(head_sha)
        and is_git_sha(marker_head_sha) then
        return {
          proposal_id = marker_issue,
          pr_number = tonumber(marker_pr),
          version = marker_version,
          head_sha = marker_head_sha,
          comment_created_at = comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.has_merged_marker(comments, issue_proposal_id, pr_number, version, head_sha)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.merged_marker(issue_proposal_id, pr_number, version, head_sha)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
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

function M.has_review_result_marker(comments, review_proposal_id, issue_proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_review_loop_marker(comments, review_proposal_id, issue_proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.review_loop_marker(review_proposal_id, issue_proposal_id, n, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function review_marker_records(comments, kind, review_proposal_id, issue_proposal_id)
  local records = {}
  if type(comments) ~= "table" then
    return records
  end

  local safe_kind = tostring(kind):gsub("%-", "%%-")
  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. safe_kind .. ":v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_issue = marker:match('issue_proposal="([^"]+)"')
      local n = tonumber(marker:match('n="(%d+)"'))
      local dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == tostring(review_proposal_id)
        and marker_issue == tostring(issue_proposal_id)
        and n ~= nil then
        table.insert(records, {
          n = n,
          dedup_key = dedup,
        })
      end
    end
  end
  return records
end

function M.has_review_loop_marker_round(comments, review_proposal_id, issue_proposal_id, n)
  for _, record in ipairs(review_marker_records(comments, "review-loop", review_proposal_id, issue_proposal_id)) do
    if record.n == n then
      return true
    end
  end
  return false
end

function M.has_review_loop_marker_dedup(comments, review_proposal_id, issue_proposal_id, dedup_key)
  for _, record in ipairs(review_marker_records(comments, "review-loop", review_proposal_id, issue_proposal_id)) do
    if record.dedup_key == tostring(dedup_key) then
      return true
    end
  end
  for _, record in ipairs(review_marker_records(comments, "review-meta-trigger", review_proposal_id, issue_proposal_id)) do
    if record.dedup_key == tostring(dedup_key) then
      return true
    end
  end
  return false
end

function M.review_loop_count_from_github_markers(comments, review_proposal_id, issue_proposal_id)
  local max_n = 0
  for _, record in ipairs(review_marker_records(comments, "review-loop", review_proposal_id, issue_proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  for _, record in ipairs(review_marker_records(comments, "review-meta-trigger", review_proposal_id, issue_proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  return max_n
end

function M.has_review_meta_trigger_marker(comments, review_proposal_id, issue_proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.review_meta_trigger_marker(review_proposal_id, issue_proposal_id, n, dedup_key)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_review_meta_marker(comments, issue_proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return false
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-meta:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      if marker_proposal == tostring(issue_proposal_id) and marker_dedup == tostring(dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.has_fix_marker(comments, issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.fix_marker(issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    if comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_any_review_result_marker(comments, review_proposal_id, issue_proposal_id)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      if marker:match('proposal="([^"]+)"') == tostring(review_proposal_id)
        and marker:match('issue_proposal="([^"]+)"') == tostring(issue_proposal_id) then
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

function M.is_safe_branch(branch)
  return is_git_ref_safe(branch)
end

function M.is_devloop_issue_branch(branch)
  return type(branch) == "string"
    and is_git_ref_safe(branch)
    and branch:find("^devloop/issue/[^/]+/.+/.+") ~= nil
end

function M.is_safe_head_sha(head_sha)
  return is_git_sha(head_sha)
end

function M.is_safe_pr_number(pr_number)
  return is_positive_pr_number(pr_number)
end

function M.is_same_repo_pr_head(pr, repo)
  if type(pr) ~= "table" then
    return false
  end
  if pr.is_cross_repository == true then
    return false
  end
  if pr.head_repository == nil then
    return false
  end
  return tostring(pr.head_repository):lower() == tostring(repo):lower()
end

function M.implementing_fact(comments, proposal_id, dedup_key)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:implementing:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_head_sha = marker:match('head_sha="([^"]+)"')
      if marker_proposal == proposal_id
        and marker_dedup == tostring(dedup_key)
        and is_git_ref_safe(marker_branch)
        and is_git_sha(marker_head_sha) then
        return {
          proposal_id = marker_proposal,
          dedup_key = marker_dedup,
          branch = marker_branch,
          head_sha = marker_head_sha,
        }
      end
    end
  end
  return nil
end

function M.pr_link_fact(comments, proposal_id)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:pr%-link:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_pr = marker:match('pr="([^"]+)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_impl_version = marker:match('impl_version="([^"]*)"')
      if marker_proposal == proposal_id
        and is_positive_pr_number(marker_pr)
        and is_git_ref_safe(marker_branch)
        and is_bounded_string(marker_impl_version, max_dedup_len) then
        return {
          proposal_id = marker_proposal,
          pr_number = tonumber(marker_pr),
          branch = marker_branch,
          impl_version = marker_impl_version,
        }
      end
    end
  end
  return nil
end

function M.pr_origin_fact(comments)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:pr%-origin:v1.-%-%->"
  for _, comment in ipairs(trusted_marker_comments(comments)) do
    for marker in comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_issue = marker:match('issue="([^"]+)"')
      local marker_branch = marker:match('branch="([^"]+)"')
      local marker_impl_version = marker:match('impl_version="([^"]*)"')
      local repo, issue_number = M.parse_proposal_id(marker_proposal)
      if repo ~= nil
        and marker_issue == issue_number
        and is_git_ref_safe(marker_branch)
        and is_bounded_string(marker_impl_version, max_dedup_len) then
        return {
          proposal_id = marker_proposal,
          repo = repo,
          issue_number = issue_number,
          branch = marker_branch,
          impl_version = marker_impl_version,
        }
      end
    end
  end
  return nil
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

function M.build_devloop_reviewing_payload(origin, pr_number, source_ref, version)
  local review_version = version or origin.impl_version
  return {
    schema = "github-devloop.reviewing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = review_version,
    dedup_key = dedup_key({
      "reviewing",
      tostring(origin.proposal_id),
      tostring(review_version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_fixing_payload(origin, pr_number, review_fact, source_ref)
  local version = origin.impl_version
  if review_fact.fix_version ~= nil then
    version = review_fact.fix_version
  end
  return {
    schema = "github-devloop.fixing.v1",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact.review_proposal_id,
    review_dedup_key = review_fact.review_dedup_key,
    reviewed_head_sha = review_fact.reviewed_head_sha,
    dedup_key = dedup_key({
      "fixing",
      tostring(origin.proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact.review_dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_devloop_review_meta_payload(unresolved, issue_proposal_id, issue_version, pr_number, n, source_ref)
  return {
    schema = "github-devloop.review-meta.v1",
    proposal_id = issue_proposal_id,
    review_proposal_id = unresolved.proposal_id,
    review_dedup_key = unresolved.dedup_key,
    version = issue_version,
    pr_number = pr_number,
    n = n,
    dedup_key = dedup_key({
      "review-meta",
      tostring(issue_proposal_id),
      tostring(issue_version),
      tostring(pr_number),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_devloop_merge_ready_payload(issue_proposal_id, pr_number, version, review_fact, source_ref)
  return {
    schema = "github-devloop.merge-ready.v1",
    proposal_id = issue_proposal_id,
    pr_number = pr_number,
    version = version,
    review_proposal_id = review_fact and review_fact.review_proposal_id,
    review_dedup_key = review_fact and review_fact.review_dedup_key,
    reviewed_head_sha = review_fact and review_fact.reviewed_head_sha,
    dedup_key = dedup_key({
      "merge-ready",
      tostring(issue_proposal_id),
      tostring(version),
      tostring(pr_number),
      tostring(review_fact and review_fact.review_dedup_key or "review"),
    }),
    source_ref = M.normalize_source_ref(source_ref),
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

function M.build_fix_prompt(fix, current_issue, review_reason)
  local prompt = require("prompts.fix")
  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(fix.proposal_id),
    review_proposal_id = M.neutralize_untrusted_prompt_text(fix.review_proposal_id),
    reviewed_head_sha = M.neutralize_untrusted_prompt_text(fix.reviewed_head_sha),
    title = M.neutralize_untrusted_prompt_text(current_issue.title),
    body = M.neutralize_untrusted_prompt_text(current_issue.body),
    review_feedback = M.neutralize_untrusted_prompt_text(review_reason),
  })
end

function M.build_review_meta_prompt(review_meta, current_issue)
  local prompt = require("prompts.review_meta")
  local comments = table.concat(M.comment_bodies(current_issue.comments), "\n\n--- comment ---\n\n")
  if #comments > max_comments_len then
    comments = comments:sub(1, max_comments_len)
  end

  return M.render_template(prompt.template, {
    proposal_id = M.neutralize_untrusted_prompt_text(review_meta.proposal_id),
    review_proposal_id = M.neutralize_untrusted_prompt_text(review_meta.review_proposal_id),
    title = M.neutralize_untrusted_prompt_text(current_issue.title),
    body = M.neutralize_untrusted_prompt_text(current_issue.body),
    comments = M.neutralize_untrusted_prompt_text(comments),
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

function M.parse_review_meta_action(stdout)
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

    if line:match("^%s*" .. action_label) ~= nil then
      local token = line:match("^%s*" .. action_label .. "%s*(%a+)%s*$")
      if token == nil or not is_review_meta_action(token:lower()) then
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

function M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, diff, source_ref)
  local review_id = M.pr_review_proposal_id(repo, pr_number, version, head_sha)
  local title = "Review PR #" .. tostring(pr_number) .. " for issue #" .. tostring(issue_number)
  if type(current_issue) == "table" and tostring(current_issue.title or "") ~= "" then
    title = "Review PR #" .. tostring(pr_number) .. ": " .. tostring(current_issue.title)
  end
  if #title > max_title_len then
    title = title:sub(1, max_title_len)
  end

  local issue_title = type(current_issue) == "table" and tostring(current_issue.title or "") or ""
  if #issue_title > max_title_len then
    issue_title = issue_title:sub(1, max_title_len)
  end
  local issue_body = type(current_issue) == "table" and tostring(current_issue.body or "") or "(issue context unavailable)"
  if issue_body == "" then
    issue_body = "(empty issue body)"
  end
  issue_title = M.neutralize_untrusted_prompt_text(neutralize_fkst_markers(issue_title))
  issue_body = M.neutralize_untrusted_prompt_text(neutralize_fkst_markers(issue_body))
  if #issue_body > max_pr_issue_context_len then
    issue_body = issue_body:sub(1, max_pr_issue_context_len)
  end
  local bounded_diff = M.neutralize_untrusted_prompt_text(neutralize_fkst_markers(M.bounded_pr_diff(diff)))
  if #bounded_diff > max_pr_diff_len then
    bounded_diff = bounded_diff:sub(1, max_pr_diff_len)
  end
  local body = "Review the PR diff and decide whether it should advance to merge-ready."
    .. "\n\n" .. untrusted_issue_data_begin
    .. "\nIssue proposal: " .. tostring(M.proposal_id(repo, issue_number))
    .. "\nReviewed PR head: " .. tostring(head_sha)
    .. "\nIssue title:\n" .. issue_title
    .. "\n\nIssue body:\n" .. issue_body
    .. "\n\nPR diff:\n" .. bounded_diff
    .. "\n" .. untrusted_issue_data_end
  if #body > max_body_len then
    error("github-devloop: PR review proposal exceeds bounded body")
  end

  return {
    schema = "consensus.proposal.v1",
    proposal_id = review_id,
    title = M.neutralize_untrusted_prompt_text(title),
    body = body,
    dedup_key = dedup_key({
      review_id,
      "review",
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_pr_review_loop_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, diff, source_ref, n)
  local proposal = M.build_pr_review_proposal(repo, issue_number, pr_number, version, head_sha, current_issue, diff, source_ref)
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
    local review_repo, pr_number = M.parse_pr_review_proposal_id(proposal.proposal_id)
    if review_repo == nil or pr_number == nil then
      return false
    end
    if not is_path_safe_key(proposal.proposal_id, max_key_len) or not is_path_safe_key(proposal.dedup_key, max_dedup_len) then
      return false
    end
  else
    if not M.is_safe_proposal_ref(proposal.proposal_id, proposal.dedup_key) then
      return false
    end
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

function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid implementing branch")
  end
  if not is_git_sha(head_sha) then
    error("github-devloop: invalid implementing head_sha")
  end
  local marker = M.implementing_marker(ready.proposal_id, ready.dedup_key, branch, head_sha)
  local state_marker = M.state_marker(ready.proposal_id, "implementing", ready.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop implementation started"
      .. "\n\nWorktree: " .. tostring(worktree)
      .. "\nBranch: " .. tostring(branch)
      .. "\nHead: " .. tostring(head_sha)
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

function M.build_pr_open_request(repo, issue_number, proposal_id, current, title, branch, head_sha)
  if type(current) ~= "table" or current.state ~= "implementing" or not is_bounded_string(current.version, max_dedup_len) then
    error("github-devloop: invalid implementing state for pr request")
  end
  if not is_git_ref_safe(branch) then
    error("github-devloop: invalid pr branch")
  end
  if not is_git_sha(head_sha) then
    error("github-devloop: invalid pr head_sha")
  end
  local bounded_title = tostring(title or "")
  if bounded_title == "" then
    bounded_title = "github-devloop implementation for #" .. tostring(issue_number)
  end
  if #bounded_title > max_pr_title_len then
    bounded_title = bounded_title:sub(1, max_pr_title_len)
  end
  local body = "github-devloop implementation PR for issue #" .. tostring(issue_number)
    .. "\n\n" .. M.pr_origin_marker(proposal_id, issue_number, branch, current.version)
  local add_labels, remove_labels = M.state_label_changes("pr-open")
  if not has_value(remove_labels, pr_authorized_label) then
    table.insert(remove_labels, pr_authorized_label)
  end
  return {
    schema = "github-proxy.pr-open.v1",
    repo = repo,
    issue_number = issue_number,
    proposal_id = proposal_id,
    impl_version = current.version,
    expected_state = current.state,
    expected_version = current.version,
    branch = branch,
    head_sha = head_sha,
    title = bounded_title,
    body = body,
    issue_comment_body_template = "github-devloop PR opened: #{{pr_number}}"
      .. "\n\n" .. M.state_marker(proposal_id, "pr-open", current.version)
      .. "\n" .. M.pr_link_marker_template(proposal_id, branch, current.version),
    issue_label_add = add_labels,
    issue_label_remove = remove_labels,
    dedup_key = dedup_key({
      "open-pr",
      tostring(proposal_id),
      tostring(current.version),
      tostring(branch),
    }),
    source_ref = {
      kind = "external",
      ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
    },
  }
end

function M.build_pr_open_comment_request(repo, issue_number, proposal_id, current, pr_number, branch, source_ref)
  local state_marker = M.state_marker(proposal_id, "pr-open", current.version)
  local link_marker = M.pr_link_marker(proposal_id, pr_number, branch, current.version)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop PR opened: #" .. tostring(pr_number)
      .. "\n\n" .. state_marker
      .. "\n" .. link_marker,
    dedup_key = dedup_key({
      "open-pr",
      "comment",
      tostring(proposal_id),
      tostring(current.version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_pr_open_label_request(repo, issue_number, proposal_id, current, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "pr-open",
    dedup_key({
      "open-pr",
      "label",
      tostring(proposal_id),
      tostring(current.version),
    }),
    source_ref
  )
end

function M.build_reviewing_comment_request(repo, issue_number, origin, pr_number, source_ref)
  local state_marker = M.state_marker(origin.proposal_id, "reviewing", origin.impl_version)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop PR is ready for review: #" .. tostring(pr_number)
      .. "\n\n" .. state_marker,
    dedup_key = dedup_key({
      "observe-pr",
      "comment",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_reviewing_label_request(repo, issue_number, origin, pr_number, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    dedup_key({
      "observe-pr",
      "label",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref
  )
end

function M.build_review_result_label_request(repo, issue_number, issue_proposal_id, reached, source_ref)
  local to_state = reached.decision == "approve" and "merge-ready" or "fixing"
  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    dedup_key({
      "review-result",
      "label",
      tostring(issue_proposal_id),
      tostring(reached.decision),
      tostring(reached.dedup_key),
    }),
    source_ref
  )
end

function M.build_review_result_comment_request(repo, issue_number, issue_proposal_id, issue_version, reached, source_ref)
  local to_state = reached.decision == "approve" and "merge-ready" or "fixing"
  local state_marker = M.state_marker(issue_proposal_id, to_state, issue_version)
  local marker = M.review_result_marker(reached.proposal_id, issue_proposal_id, reached.decision, reached.dedup_key)
  local merge_marker = ""
  if reached.decision == "approve" then
    local _, pr_number, _, reviewed_head_sha = M.parse_pr_review_proposal_id(reached.proposal_id)
    merge_marker = "\n" .. M.merge_ready_marker(issue_proposal_id, pr_number, issue_version, reached.proposal_id, reached.dedup_key, reviewed_head_sha)
  end
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop PR review decision: " .. tostring(reached.decision)
      .. "\n\n" .. body_text
      .. "\n\n" .. state_marker
      .. "\n" .. marker
      .. merge_marker,
    dedup_key = dedup_key({
      "review-result",
      "comment",
      tostring(issue_proposal_id),
      tostring(reached.decision),
      tostring(reached.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, source_ref)
  local safe_reason = M.sanitize_key(reason or "gate-failed", false):gsub("/", "-")
  local state_marker = M.state_marker(merge_ready.proposal_id, "fixing", fix_version)
  local marker = M.merge_gate_marker(
    merge_ready.proposal_id,
    merge_ready.pr_number,
    fix_version,
    merge_ready.review_proposal_id,
    merge_ready.review_dedup_key,
    merge_ready.reviewed_head_sha,
    safe_reason
  )
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop merge gate failed: " .. safe_reason
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = dedup_key({
      "merge",
      "comment",
      "fixing",
      tostring(merge_ready.proposal_id),
      tostring(merge_ready.version),
      safe_reason,
    }),
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_fix_reviewing_label_request(repo, issue_number, fix, new_head_sha, new_version)
  local request = M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    dedup_key({
      "fix",
      "label",
      tostring(fix.proposal_id),
      tostring(fix.review_dedup_key),
      tostring(new_head_sha),
    }),
    fix.source_ref
  )
  if not has_value(request.remove_labels, fix_authorized_label) then
    table.insert(request.remove_labels, fix_authorized_label)
  end
  return request
end

function M.build_fix_reviewing_comment_request(repo, issue_number, fix, old_head_sha, new_head_sha, new_version)
  local state_marker = M.state_marker(fix.proposal_id, "reviewing", new_version or fix.version)
  local marker = M.fix_marker(fix.proposal_id, fix.review_proposal_id, fix.review_dedup_key, old_head_sha, new_head_sha)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop fix pushed for re-review"
      .. "\n\nPrevious reviewed head: " .. tostring(old_head_sha)
      .. "\nNew head: " .. tostring(new_head_sha)
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = dedup_key({
      "fix",
      "comment",
      tostring(fix.proposal_id),
      tostring(fix.review_dedup_key),
      tostring(new_head_sha),
    }),
    source_ref = M.normalize_source_ref(fix.source_ref),
  }
end

function M.build_fix_review_meta_label_request(repo, issue_number, fix, reason)
  local request = M.build_state_label_request(
    repo,
    issue_number,
    "review-meta",
    dedup_key({
      "fix",
      "label",
      "review-meta",
      tostring(reason or "no-fix"),
      tostring(fix.review_dedup_key),
    }),
    fix.source_ref
  )
  if not has_value(request.remove_labels, fix_authorized_label) then
    table.insert(request.remove_labels, fix_authorized_label)
  end
  return request
end

function M.build_fix_review_meta_comment_request(repo, issue_number, fix, reason, detail)
  local safe_reason = M.sanitize_key(reason or "no-fix"):gsub("/", "-")
  local text = tostring(detail or "")
  if #text > max_impl_output_len then
    text = text:sub(1, max_impl_output_len)
  end
  if text == "" then
    text = "(no fix output)"
  end
  text = M.neutralize_untrusted_comment_text(text)
  local state_marker = M.state_marker(fix.proposal_id, "review-meta", fix.version)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop fix escalated to review-meta: " .. safe_reason
      .. "\n\n" .. text
      .. "\n\n" .. state_marker,
    dedup_key = dedup_key({
      "fix",
      "comment",
      "review-meta",
      safe_reason,
      tostring(fix.dedup_key),
    }),
    source_ref = M.normalize_source_ref(fix.source_ref),
  }
end

function M.build_review_loop_comment_request(repo, issue_number, unresolved, issue_proposal_id, n, source_ref)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop PR review no-consensus loop: " .. tostring(n)
      .. "\n\n" .. M.review_loop_marker(unresolved.proposal_id, issue_proposal_id, n, unresolved.dedup_key),
    dedup_key = dedup_key({
      "review-loop",
      "comment",
      tostring(issue_proposal_id),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_review_meta_trigger_label_request(repo, issue_number, unresolved, issue_proposal_id, n, source_ref)
  local request = M.build_state_label_request(
    repo,
    issue_number,
    "review-meta",
    dedup_key({
      "review-loop",
      "label",
      "review-meta",
      tostring(issue_proposal_id),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref or unresolved.source_ref
  )
  if not has_value(request.remove_labels, fix_authorized_label) then
    table.insert(request.remove_labels, fix_authorized_label)
  end
  return request
end

function M.build_review_meta_trigger_comment_request(repo, issue_number, unresolved, issue_proposal_id, issue_version, n, source_ref)
  local state_marker = M.state_marker(issue_proposal_id, "review-meta", issue_version)
  local marker = M.review_meta_trigger_marker(unresolved.proposal_id, issue_proposal_id, n, unresolved.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop PR review unresolved: escalating to review-meta after " .. tostring(n) .. " attempts"
      .. "\n\n" .. state_marker
      .. "\n" .. marker,
    dedup_key = dedup_key({
      "review-loop",
      "comment",
      "review-meta",
      tostring(issue_proposal_id),
      tostring(n),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }
end

function M.build_review_meta_label_request(repo, issue_number, review_meta, action, version)
  local to_state = action == "fix" and "fixing" or action == "accept" and "merge-ready" or "blocked"
  local request = M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    dedup_key({
      "review-meta",
      "label",
      tostring(action),
      tostring(review_meta.dedup_key),
      tostring(version or review_meta.version),
    }),
    review_meta.source_ref
  )
  if to_state ~= "fixing" and not has_value(request.remove_labels, fix_authorized_label) then
    table.insert(request.remove_labels, fix_authorized_label)
  end
  return request
end

function M.build_review_meta_comment_request(repo, issue_number, review_meta, action, reason, version)
  local to_state = action == "fix" and "fixing" or action == "accept" and "merge-ready" or "blocked"
  local safe_reason = M.neutralize_untrusted_comment_text(reason or "")
  local state_version = version or review_meta.version
  local merge_marker = ""
  if action == "accept" then
    local _, _, _, reviewed_head_sha = M.parse_pr_review_proposal_id(review_meta.review_proposal_id)
    merge_marker = "\n" .. M.merge_ready_marker(review_meta.proposal_id, review_meta.pr_number, state_version, review_meta.review_proposal_id, review_meta.dedup_key, reviewed_head_sha)
  end
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop review-meta action: " .. tostring(action)
      .. "\n\nReason:\n" .. safe_reason
      .. "\n\n" .. M.state_marker(review_meta.proposal_id, to_state, state_version)
      .. "\n" .. M.review_meta_marker(review_meta.proposal_id, review_meta.dedup_key, action, state_version)
      .. merge_marker,
    dedup_key = dedup_key({
      "review-meta",
      "comment",
      tostring(review_meta.dedup_key),
      tostring(state_version),
    }),
    source_ref = M.normalize_source_ref(review_meta.source_ref),
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

function M.is_supported_pr(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "pr"
    and payload.repo ~= nil
    and M.is_safe_pr_number(payload.number)
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

function M.is_supported_review_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and (payload.decision == "approve" or payload.decision == "reject")
    and M.is_safe_pr_review_result_ref(payload.proposal_id, payload.dedup_key)
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

function M.is_supported_pr_review_unresolved(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_unresolved.v1"
    and M.is_safe_pr_review_result_ref(payload.proposal_id, payload.dedup_key)
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

function M.is_supported_reviewing(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.reviewing.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and is_bounded_string(payload.version, max_dedup_len)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_fixing(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.fixing.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and is_bounded_string(payload.version, max_dedup_len)
    and M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    and is_git_sha(payload.reviewed_head_sha)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_review_meta(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.review-meta.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    and is_bounded_string(payload.version, max_dedup_len)
    and M.is_safe_pr_number(payload.pr_number)
    and tonumber(payload.n) ~= nil
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_merge_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.merge-ready.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and is_bounded_string(payload.version, max_dedup_len)
    and M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    and is_git_sha(payload.reviewed_head_sha)
    and has_bounded_source_ref(payload.source_ref)
end

return M
