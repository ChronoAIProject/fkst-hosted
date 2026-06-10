local S = {}

function S.install(M)

local max_key_len = 200
local max_dedup_len = 512
local max_title_len = 240
local max_body_len = 12000
local max_comments_len = 12000
local max_meta_reason_len = 2000
local max_framing_len = 1000
local max_impl_output_len = 2000
local max_blocking_gap_len = 240
local max_review_ledger_len = 1200
local max_pr_issue_context_len = 3000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50
local max_version_key_len = 40
local max_worktree_prefix_len = 90
local max_branch_len = 160
local max_sha_len = 64
local max_pr_title_len = 240
local max_judgment_prefix_len = 120
local action_label = "⟦FKST:ACTION⟧"
local intake_label = "⟦FKST:INTAKE⟧"
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
local pr_open_label = "fkst-dev:pr-open"
local reviewing_label = "fkst-dev:reviewing"
local merge_ready_label = "fkst-dev:merge-ready"
local merging_label = "fkst-dev:merging"
local merged_label = "fkst-dev:merged"
local fixing_label = "fkst-dev:fixing"
local review_meta_label = "fkst-dev:review-meta"
local impl_failed_label = "fkst-dev:impl-failed"
local blocked_label = "fkst-dev:blocked"
local blocked_on_dependency_label = "fkst-dev:blocked-on-dependency"

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
}

local state_by_label = {}
for state, label in pairs(label_by_state) do
  state_by_label[label] = state
end

local state_graph = {
  unmanaged = { "thinking" },
  thinking = { "ready", "blocked" },
  ready = { "implementing" },
  implementing = { "pr-open", "impl-failed" },
  ["pr-open"] = { "reviewing" },
  reviewing = { "merge-ready", "fixing", "review-meta" },
  ["merge-ready"] = { "reviewing", "merging", "fixing", "blocked" },
  merging = { "merged", "fixing", "blocked" },
  merged = {},
  fixing = { "reviewing", "review-meta" },
  ["review-meta"] = { "fixing", "blocked" },
  ["impl-failed"] = {},
  blocked = {},
}

local state_order = { "thinking", "ready", "implementing", "pr-open", "reviewing", "merge-ready", "fixing", "impl-failed", "blocked", "review-meta", "merging", "merged" }
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
local trusted_bot_login = nil
local comment_body
local comment_author_login
local is_trusted_comment

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

local function is_review_meta_action(value)
  return value == "fix" or value == "block"
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

function M.parse_issue_source_ref(source_ref)
  if type(source_ref) ~= "table" or source_ref.kind ~= "external" then
    return nil
  end
  local ref = tostring(source_ref.ref or "")
  local issue_number = ref:match("#issue/(%d+)$")
  local repo = issue_number and ref:sub(1, #ref - #("#issue/" .. issue_number)) or nil
  if repo == nil or repo == "" or not is_positive_pr_number(issue_number) then
    return nil
  end
  if not M.issue_ref_round_trips(repo, issue_number) then
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

function M.intake_dedup_key(proposal_id, updated_at)
  return M._dedup_key({
    "intake",
    tostring(proposal_id),
    M.safe_updated_at(updated_at or "unknown"),
  })
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

function M.judgment_worktree_path(runtime_root, role, identity)
  local root = trim(runtime_root)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: invalid FKST_RUNTIME_ROOT")
  end
  local slug = M.sanitize_key(tostring(role or "") .. "-" .. tostring(identity or ""), false):gsub("/", "-")
  slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", ""):gsub("%.+$", "")
  if slug == "" then
    slug = "judgment"
  end
  if #slug > max_judgment_prefix_len then
    slug = slug:sub(1, max_judgment_prefix_len):gsub("%-+$", ""):gsub("%.+$", "")
  end
  if slug == "" then
    slug = "judgment"
  end
  local suffix = decimal_checksum(tostring(role) .. "#" .. tostring(identity))
  return root:gsub("/+$", "") .. "/judgment-worktrees/github-devloop-" .. slug .. "-" .. suffix
end

function M.judgment_codex_opts(prompt, worktree)
  return {
    prompt = prompt,
    worktree = worktree,
    sandbox = "read-only",
  }
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
    local sentinel_line = line:match("^%s*[+%- ]?%s*(.+)$") or line
    if sentinel_line:match("^%s*" .. action_label) ~= nil
      or sentinel_line:match("^%s*" .. reason_label) ~= nil
      or sentinel_line:match("^%s*" .. intake_label) ~= nil
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

function M.quote_untrusted_prompt_text(text)
  local value = M._neutralize_fkst_markers(text)
  local output = {}
  local start = 1
  while true do
    local newline = value:find("\n", start, true)
    if newline == nil then
      table.insert(output, "> " .. value:sub(start))
      break
    end

    table.insert(output, "> " .. value:sub(start, newline - 1))
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


function M.trusted_bot_login()
  return trusted_bot_login or test_bot_login
end

M._max_key_len = max_key_len
M._max_dedup_len = max_dedup_len
M._max_title_len = max_title_len
M._max_body_len = max_body_len
M._max_comments_len = max_comments_len
M._max_meta_reason_len = max_meta_reason_len
M._max_framing_len = max_framing_len
M._max_impl_output_len = max_impl_output_len
M._max_blocking_gap_len = max_blocking_gap_len
M._max_review_ledger_len = max_review_ledger_len
M._max_pr_issue_context_len = max_pr_issue_context_len
M._max_pr_title_len = max_pr_title_len
M._action_label = action_label
M._intake_label = intake_label
M._reason_label = reason_label
M._verdict_label = verdict_label
M._reply_label = reply_label
M._untrusted_issue_data_begin = untrusted_issue_data_begin
M._untrusted_issue_data_end = untrusted_issue_data_end
M._test_bot_login = test_bot_login
M._enabled_label = enabled_label
M._thinking_label = thinking_label
M._ready_label = ready_label
M._implementing_label = implementing_label
M._pr_open_label = pr_open_label
M._reviewing_label = reviewing_label
M._merge_ready_label = merge_ready_label
M._merging_label = merging_label
M._merged_label = merged_label
M._fixing_label = fixing_label
M._review_meta_label = review_meta_label
M._impl_failed_label = impl_failed_label
M._blocked_label = blocked_label
M._blocked_on_dependency_label = blocked_on_dependency_label
M._state_labels = state_labels
M._label_by_state = label_by_state
M._state_graph = state_graph
M._state_order = state_order
M._state_stage_rank = state_stage_rank
M._shell_single_quote = shell_single_quote
M._trim = trim
M._neutralize_fkst_markers = neutralize_fkst_markers
M._one_line = one_line
M._decimal_checksum = decimal_checksum
M._is_bounded_string = is_bounded_string
M._has_value = has_value
M._is_review_meta_action = is_review_meta_action
M._is_path_safe_key = is_path_safe_key
M._is_git_ref_safe = is_git_ref_safe
M._is_git_sha = is_git_sha
M._is_positive_pr_number = is_positive_pr_number
M._has_bounded_source_ref = has_bounded_source_ref
M._dedup_key = dedup_key
end

return S
