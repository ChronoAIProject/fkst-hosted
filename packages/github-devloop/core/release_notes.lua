local S = {}

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
  while #lines > 0 and M._trim(lines[1]) == "" do
    table.remove(lines, 1)
  end
  while #lines > 0 and M._trim(lines[#lines]) == "" do
    table.remove(lines)
  end
  return table.concat(lines, "\n")
end

local function strip_sentinel(text)
  local lines = {}
  for line in (tostring(text or "") .. "\n"):gmatch("(.-)\n") do
    if M._trim(line) ~= ai_sentinel then
      table.insert(lines, line)
    end
  end
  return table.concat(lines, "\n")
end

function M.release_notes_fallback_body(upstream, integration, ahead)
  return table.concat({
    "Automated rollup from `" .. tostring(integration) .. "` into `" .. tostring(upstream) .. "`.",
    "",
    "Ahead commits: " .. tostring(ahead),
    "Merge policy: CI green and mergeable current PR facts.",
    "",
    "Zh: zi dong hui zong `" .. tostring(integration) .. "` to `" .. tostring(upstream) .. "`; publish still depends on current PR facts and CI.",
    ai_sentinel,
  }, "\n")
end

function M.normalize_release_notes(stdout)
  local body = normalize_lines(strip_sentinel(M._neutralize_fkst_markers(stdout)))
  if body == "" then
    body = "Automated rollup release notes."
  end
  local suffix = "\n" .. ai_sentinel
  local limit = max_release_notes_len - #suffix
  body = bounded(body, limit)
  body = body:gsub("%s+$", "")
  if body == "" then
    body = "Automated rollup release notes."
  end
  return body .. suffix
end

function M.build_release_notes_prompt(repo, upstream, integration, head_sha, ahead)
  local prompt = require("prompts.release_notes")
  return M.render_template(prompt.template, {
    repo = M.neutralize_untrusted_prompt_text(repo),
    upstream_branch = M.neutralize_untrusted_prompt_text(upstream),
    integration_branch = M.neutralize_untrusted_prompt_text(integration),
    head_sha = M.neutralize_untrusted_prompt_text(head_sha),
    ahead = M.neutralize_untrusted_prompt_text(ahead),
    max_bytes = tostring(max_release_notes_len),
    ai_sentinel = ai_sentinel,
  })
end

function M.release_notes_publish_policy(cfg)
  if type(cfg) ~= "table" then
    error("github-devloop: release notes publish policy requires config")
  end
  return {
    allow_fallback = cfg.write_mode == "real",
    write_mode = tostring(cfg.write_mode or ""),
  }
end

function M.draft_release_notes(args)
  local policy = args.publish_policy
  if type(policy) ~= "table" then
    error("github-devloop: release notes publish policy is required")
  end
  local result = spawn_codex_sync({
    prompt = M.build_release_notes_prompt(
      args.repo,
      args.upstream_branch,
      args.integration_branch,
      args.head_sha,
      args.ahead
    ),
    timeout = 3600,
  })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    if policy.allow_fallback == true then
      return M.release_notes_fallback_body(args.upstream_branch, args.integration_branch, args.ahead), "fallback"
    end
    local stderr = type(result) == "table" and result.stderr or "missing codex result"
    error("github-devloop: release notes codex failed: " .. tostring(stderr))
  end
  return M.normalize_release_notes(result.stdout), "codex"
end

M._max_release_notes_len = max_release_notes_len
M._release_notes_ai_sentinel = ai_sentinel
end

return S
