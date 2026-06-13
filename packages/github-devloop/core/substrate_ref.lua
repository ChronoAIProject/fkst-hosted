local S = {}

function S.install(M)
local substrate_ref_path = ".fkst/substrate-ref"
local substrate_remote = "https://github.com/ChronoAIProject/fkst-substrate.git"
local substrate_branch = "dev"
local bump_branch = "chore/substrate-ref-bump"
local bump_title = "chore: bump fkst-substrate pin"

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or M.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for substrate-ref scan")
  end
  return value
end

local function run_cmd(cmd, timeout, label)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function run_gh(cmd, timeout, label)
  local result = M.gh_exec({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(label) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function read_runtime_root()
  local result = run_cmd(M.read_runtime_root_cmd(), 30, "runtime root read")
  local root = M._trim(result.stdout)
  if root == "" or root:find("[\r\n]") ~= nil then
    error("github-devloop: FKST_RUNTIME_ROOT is required for substrate-ref bump")
  end
  return root:gsub("/+$", "")
end

local function read_pin()
  local text = file.read(substrate_ref_path)
  local pin = M._trim(text)
  if not M._is_git_sha(pin) then
    error("github-devloop: invalid .fkst/substrate-ref pin")
  end
  return pin:lower()
end

local function parse_ls_remote(stdout)
  local sha, ref = tostring(stdout or ""):match("^(%x+)%s+(refs/heads/[^%s]+)")
  if ref ~= "refs/heads/" .. substrate_branch or not M._is_git_sha(sha) then
    return nil
  end
  return sha:lower()
end

local function fetch_substrate_dev_head()
  local result = run_cmd(
    M.git_ls_remote_branch_cmd(substrate_remote, substrate_branch),
    60,
    "git ls-remote fkst-substrate dev"
  )
  local sha = parse_ls_remote(result.stdout)
  if sha == nil then
    error("github-devloop: git ls-remote did not return a valid fkst-substrate dev head")
  end
  return sha
end

local function parse_pr_list(stdout)
  local pages = json.decode(stdout)
  local prs = {}
  if type(pages) ~= "table" then
    return prs
  end
  for _, page in ipairs(pages) do
    if type(page) == "table" then
      for _, pr in ipairs(page) do
        if type(pr) == "table" and pr.number ~= nil then
          table.insert(prs, pr)
        end
      end
    end
  end
  return prs
end

local function existing_bump_pr(repo)
  local result = run_gh(M.gh_pr_list_head_cmd(repo, bump_branch), 30, "gh substrate-ref PR list")
  local prs = parse_pr_list(result.stdout)
  if #prs > 1 then
    error("github-devloop: multiple open substrate-ref bump PRs found")
  end
  return prs[1]
end

local function bump_worktree_path(runtime_root, repo, head_sha)
  local slug = M.sanitize_key("substrate-ref-" .. tostring(repo), false):gsub("/", "-")
  slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
  if slug == "" then
    slug = "substrate-ref"
  end
  if #slug > 90 then
    slug = slug:sub(1, 90):gsub("%-+$", "")
  end
  return runtime_root .. "/worktrees/" .. slug .. "-" .. tostring(head_sha):sub(1, 12)
end

local function write_pr_body(repo, current_pin, target_sha)
  local path = "/tmp/fkst-github-devloop-substrate-ref-bump-" .. M.safe_repo(repo):gsub("/", "-") .. ".md"
  local body = table.concat({
    "Updates `.fkst/substrate-ref` to the current `fkst-substrate` `dev` head.",
    "",
    "- Previous pin: `" .. tostring(current_pin) .. "`",
    "- New pin: `" .. tostring(target_sha) .. "`",
    "",
    "CI is the verification gate for package compatibility with the new engine pin.",
    "",
    "⟦AI:FKST⟧",
  }, "\n")
  file.write(path, body .. "\n")
  return path
end

local function ensure_clean_pin_delta(worktree)
  local diff = run_cmd("git -C " .. M._shell_single_quote(worktree) .. " diff --name-only HEAD", 30, "git diff name-only")
  local name = M._trim(diff.stdout)
  if name ~= substrate_ref_path then
    error("github-devloop: substrate-ref bump changed unexpected paths")
  end
end

local function fetch_bump_branch_head()
  local fetch = exec_sync({ cmd = M.git_fetch_branch_cmd("origin", bump_branch), timeout = 60 })
  if fetch.exit_code ~= 0 then
    return nil
  end
  local head = run_cmd(M.git_remote_branch_head_cmd("origin", bump_branch), 30, "git substrate-ref bump branch head")
  local sha = M._trim(head.stdout)
  if not M._is_git_sha(sha) then
    error("github-devloop: invalid substrate-ref bump branch head")
  end
  return sha:lower()
end

local function create_or_update_branch(repo, base_branch, current_pin, target_sha)
  local base_head = M.current_base_head(base_branch)
  if base_head == nil then
    error("github-devloop: unable to read base branch head for substrate-ref bump")
  end
  local runtime_root = read_runtime_root()
  local worktree = bump_worktree_path(runtime_root, repo, target_sha)
  local old_branch_head = fetch_bump_branch_head()
  run_cmd(M.git_worktree_remove_if_present_cmd(worktree), 60, "git stale substrate-ref worktree remove")
  run_cmd(M.git_worktree_add_reset_branch_cmd(worktree, bump_branch, base_head), 120, "git substrate-ref worktree add")
  run_cmd(M.git_write_file_cmd(worktree, substrate_ref_path, target_sha .. "\n"), 30, "write substrate-ref pin")
  ensure_clean_pin_delta(worktree)
  run_cmd(M.git_add_all_cmd(worktree), 30, "git substrate-ref add")
  run_cmd(M.git_commit_cmd(worktree, "chore: bump fkst-substrate pin"), 60, "git substrate-ref commit")
  if old_branch_head == nil then
    run_cmd(M.git_push_worktree_branch_update_cmd(worktree, bump_branch), 120, "git substrate-ref push")
  else
    run_cmd(
      M.git_push_worktree_branch_update_with_lease_cmd(worktree, bump_branch, old_branch_head),
      120,
      "git substrate-ref push"
    )
  end
  run_cmd(M.git_worktree_remove_cmd(worktree), 60, "git substrate-ref worktree remove")
end

local function create_pr(repo, base_branch, current_pin, target_sha)
  local body_file = write_pr_body(repo, current_pin, target_sha)
  run_gh(M.gh_pr_create_cmd(repo, bump_branch, base_branch, bump_title, body_file), 60, "gh substrate-ref PR create")
end

local function log_scan(action, fields)
  local parts = { "action=" .. tostring(action) }
  for _, field in ipairs(fields or {}) do
    table.insert(parts, tostring(field))
  end
  M.log_line("info", "substrate_ref_scan", "repo-management-plane", "SUBSTRATE_REF", parts)
end

function M.substrate_ref_constants()
  return {
    path = substrate_ref_path,
    remote = substrate_remote,
    branch = substrate_branch,
    bump_branch = bump_branch,
    title = bump_title,
  }
end

function M.substrate_ref_scan()
  local cfg = M.devloop_config()
  local repo = require_repo(cfg.repo)
  if cfg.write_mode == "real" then
    M.assert_trusted_bot_configured()
  end

  local current_pin = read_pin()
  local target_sha = fetch_substrate_dev_head()
  if current_pin == target_sha then
    log_scan("unchanged", {
      "repo=" .. repo,
      "pin=" .. current_pin,
    })
    return { status = "current", pin = current_pin, target = target_sha }
  end

  local existing = existing_bump_pr(repo)
  if cfg.write_mode ~= "real" then
    log_scan("bump-planned", {
      "mode=" .. cfg.write_mode,
      "repo=" .. repo,
      "from=" .. current_pin,
      "to=" .. target_sha,
      "branch=" .. bump_branch,
      "existing_pr=" .. tostring(existing and existing.number or ""),
    })
    return {
      status = "planned",
      pin = current_pin,
      target = target_sha,
      existing_pr = existing and existing.number or nil,
      branch = bump_branch,
    }
  end

  with_lock("github-devloop/substrate-ref/" .. M.safe_repo(repo), function()
    create_or_update_branch(repo, cfg.upstream_branch, current_pin, target_sha)
    if existing == nil then
      create_pr(repo, cfg.upstream_branch, current_pin, target_sha)
      log_scan("pr-created", {
        "mode=real",
        "repo=" .. repo,
        "from=" .. current_pin,
        "to=" .. target_sha,
        "branch=" .. bump_branch,
      })
    else
      log_scan("pr-updated", {
        "mode=real",
        "repo=" .. repo,
        "from=" .. current_pin,
        "to=" .. target_sha,
        "branch=" .. bump_branch,
        "pr=" .. tostring(existing.number),
      })
    end
  end)

  return {
    status = existing == nil and "created" or "updated",
    pin = current_pin,
    target = target_sha,
    existing_pr = existing and existing.number or nil,
    branch = bump_branch,
  }
end
end

return S
