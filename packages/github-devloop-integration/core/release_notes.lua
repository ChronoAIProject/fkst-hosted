local devloop_base = require("devloop.base")
local strings = require("contract.strings")
local S = {}
local forge_validators = require("devloop.forge_validators")
local github_factory = require("devloop.github_factory")
local github_view = require("forge.github_view")

function S.install(M)
local max_release_notes_len = 4000
local ai_sentinel = string.char(226, 159, 166) .. "AI:FKST" .. string.char(226, 159, 167)

local function bounded(value, limit)
  local text = tostring(value or "")
  if #text > limit then
    text = text:sub(1, limit)
  end
  return text
end

local function normalize_lines(text)
  local lines = {}
  for line in (tostring(text or "") .. "\n"):gmatch("(.-)\n") do
    table.insert(lines, (line:gsub("%s+$", "")))
  end
  while #lines > 0 and strings.trim(lines[1]) == "" do
    table.remove(lines, 1)
  end
  while #lines > 0 and strings.trim(lines[#lines]) == "" do
    table.remove(lines)
  end
  return table.concat(lines, "\n")
end

local function strip_sentinel(text)
  local lines = {}
  for line in (tostring(text or "") .. "\n"):gmatch("(.-)\n") do
    if strings.trim(line) ~= ai_sentinel then
      table.insert(lines, line)
    end
  end
  return table.concat(lines, "\n")
end

local function utf8(...)
  return string.char(...)
end

local function json_string_array(values)
  local encoded = {}
  for _, value in ipairs(values or {}) do
    table.insert(encoded, github_view.json_value(value))
  end
  return "[" .. table.concat(encoded, ",") .. "]"
end

local function comments_json(comments)
  local encoded = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(encoded, "{"
      .. '"author_login":' .. github_view.json_value(comment.author_login)
      .. ',"body":' .. github_view.json_value(comment.body)
      .. ',"created_at":' .. github_view.json_value(comment.created_at)
      .. "}")
  end
  return "[" .. table.concat(encoded, ",") .. "]"
end

local function referenced_issue_json(issue)
  return "{"
    .. '"number":' .. github_view.json_value(issue.number)
    .. ',"title":' .. github_view.json_value(issue.title)
    .. ',"body":' .. github_view.json_value(issue.body)
    .. ',"state":' .. github_view.json_value(issue.state)
    .. ',"labels":' .. json_string_array(issue.labels)
    .. ',"comments":' .. comments_json(issue.comments)
    .. "}"
end

local function referenced_numbers(commit_history)
  local numbers = {}
  local seen = {}
  for line in (tostring(commit_history or "") .. "\n"):gmatch("(.-)\n") do
    local subject = line:match("^[^\t]*\t(.*)$") or line
    for number in subject:gmatch("#(%d+)") do
      local normalized = tonumber(number)
      if normalized ~= nil and normalized >= 1 and seen[normalized] ~= true then
        seen[normalized] = true
        table.insert(numbers, normalized)
      end
    end
  end
  return numbers
end

local function prefetch_release_notes_source(args)
  local git = args.git or M.git
  local github = args.github or github_factory.production_handle()
  if type(git) ~= "table" or type(git.log_subjects_between_remote_branch) ~= "function" then
    error("github-devloop: release-notes-git-port-missing: release notes require a git adapter")
  end
  if type(github) ~= "table" or type(github.read_issue) ~= "function" then
    error("github-devloop: release-notes-github-port-missing: release notes require a GitHub adapter")
  end

  local history = git.log_subjects_between_remote_branch(args.upstream_branch, args.head_sha, 60)
  if type(history) ~= "table" or tonumber(history.exit_code) ~= 0 then
    local stderr = type(history) == "table" and history.stderr or "missing git result"
    error("github-devloop: release-notes-git-log-failed: failed to read release notes history: " .. tostring(stderr))
  end

  local issues = {}
  for _, number in ipairs(referenced_numbers(history.stdout)) do
    local issue = github.read_issue({
      kind = "external",
      ref = tostring(args.repo) .. "#issue/" .. tostring(number),
    }, {
      consumer = "github-devloop-integration.release_notes",
      force_fresh = true,
      timeout = 30,
    })
    table.insert(issues, referenced_issue_json(issue))
  end
  return {
    commit_history = tostring(history.stdout or ""):gsub("%s+$", ""),
    referenced_github_context = "[" .. table.concat(issues, ",") .. "]",
  }
end

local function zh_summary_label()
  return utf8(228, 184, 173, 230, 150, 135, 230, 145, 152, 232, 166, 129) .. ": "
end

local function zh_fallback_sentence(integration, upstream)
  return utf8(232, 135, 170, 229, 138, 168, 229, 176, 134) .. " `"
    .. tostring(integration)
    .. "` "
    .. utf8(230, 177, 135, 230, 128, 187, 229, 136, 176)
    .. " `"
    .. tostring(upstream)
    .. "`; "
    .. utf8(
      229, 143, 145, 229, 184, 131, 228, 187, 141, 228, 190, 157,
      232, 181, 150, 229, 189, 147, 229, 137, 141, 32, 80, 82, 32,
      228, 186, 139, 229, 174, 158, 227, 128, 129, 67, 73, 32,
      228, 184, 142, 229, 143, 175, 229, 144, 136, 229, 185, 182,
      231, 138, 182, 230, 128, 129
    )
    .. "."
end

function M.release_notes_fallback_body(upstream, integration, ahead)
  local body = table.concat({
    "Automated rollup from `" .. tostring(integration) .. "` into `" .. tostring(upstream) .. "`.",
    "",
    "Ahead commits: " .. tostring(ahead),
    "Merge policy: CI green and mergeable current PR facts.",
    "",
    zh_summary_label() .. zh_fallback_sentence(integration, upstream),
  }, "\n")
  return M.normalize_release_notes(body)
end

function M.normalize_release_notes(stdout)
  local body = normalize_lines(strip_sentinel(devloop_base._neutralize_fkst_markers(stdout)))
  if body == "" then
    error("github-devloop: release-notes-body-empty: release notes body is empty")
  end
  local suffix = "\n" .. ai_sentinel
  local limit = max_release_notes_len - #suffix
  body = bounded(body, limit)
  body = body:gsub("%s+$", "")
  if body == "" then
    error("github-devloop: release-notes-body-empty: release notes body is empty")
  end
  return body .. suffix
end

function M.build_release_notes_prompt(args)
  if type(args) ~= "table" then
    error("github-devloop: release-notes-prompt-invalid: release notes prompt requires source data")
  end
  local prompt = require("prompts.release_notes")
  return devloop_base.render_template(prompt.template, {
    repo = devloop_base.neutralize_untrusted_prompt_text(args.repo),
    upstream_branch = devloop_base.neutralize_untrusted_prompt_text(args.upstream_branch),
    integration_branch = devloop_base.neutralize_untrusted_prompt_text(args.integration_branch),
    head_sha = devloop_base.neutralize_untrusted_prompt_text(args.head_sha),
    ahead = devloop_base.neutralize_untrusted_prompt_text(args.ahead),
    commit_history = devloop_base.neutralize_untrusted_prompt_text(args.commit_history),
    referenced_github_context = devloop_base.neutralize_untrusted_prompt_text(args.referenced_github_context),
    max_bytes = tostring(max_release_notes_len),
    ai_sentinel = ai_sentinel,
  })
end

function M.release_notes_publish_policy(cfg)
  if type(cfg) ~= "table" then
    error("github-devloop: release-notes-policy-invalid: release notes publish policy requires config")
  end
  return {
    allow_fallback = cfg.allow_release_notes_fallback == true,
    write_mode = tostring(cfg.write_mode or ""),
  }
end

function M.gh_pr_create_body_cmd(repo, head, base, title, body)
  error("github-devloop: adapter-only: release notes PR create uses forge.github adapter")
end

function M.gh_pr_create_body(repo, head, base, title, body, timeout)
  if not forge_validators.is_git_ref_safe(head) then
    error("github-devloop: git-ref-invalid: invalid PR head branch")
  end
  if not forge_validators.is_git_ref_safe(base) then
    error("github-devloop: git-ref-invalid: invalid PR base branch")
  end
  local normalized_body = M.normalize_release_notes(body)
  normalized_body = M.with_github_debug_stamp(normalized_body, {
    emitter = "github-devloop.rollup.pr-create",
    target = "pr:" .. tostring(repo) .. "#new",
    dedup_key = tostring(head) .. "->" .. tostring(base),
  })
  local ok, result_or_error = pcall(function()
    return github_factory.production_handle().pr_create_body(repo, head, base, title, normalized_body, timeout or 60)
  end)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error(result_or_error)
end

function M.draft_release_notes(args)
  local policy = args.publish_policy
  if type(policy) ~= "table" then
    error("github-devloop: release-notes-policy-missing: release notes publish policy is required")
  end
  local source_ok, source = pcall(prefetch_release_notes_source, args)
  if not source_ok then
    if policy.allow_fallback == true then
      return M.release_notes_fallback_body(args.upstream_branch, args.integration_branch, args.ahead), "fallback"
    end
    error(source)
  end
  local result = spawn_codex_sync({
    prompt = M.build_release_notes_prompt({
      repo = args.repo,
      upstream_branch = args.upstream_branch,
      integration_branch = args.integration_branch,
      head_sha = args.head_sha,
      ahead = args.ahead,
      commit_history = source.commit_history,
      referenced_github_context = source.referenced_github_context,
    }),
    sandbox = "read-only",
    timeout = 3600,
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    if policy.allow_fallback == true then
      return M.release_notes_fallback_body(args.upstream_branch, args.integration_branch, args.ahead), "fallback"
    end
    local stderr = type(result) == "table" and result.stderr or "missing codex result"
    error("github-devloop: release-notes-codex-failed: release notes codex failed: " .. tostring(stderr))
  end
  local ok, normalized = pcall(M.normalize_release_notes, result.stdout)
  if not ok then
    if policy.allow_fallback == true then
      return M.release_notes_fallback_body(args.upstream_branch, args.integration_branch, args.ahead), "fallback"
    end
    error(normalized)
  end
  return normalized, "codex"
end

M._max_release_notes_len = max_release_notes_len
M._release_notes_ai_sentinel = ai_sentinel
end

return S
