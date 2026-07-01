local base_ids = require("devloop.base_ids")
local BranchTrain = {}
local strings = require("contract.strings")
local forge_validators = require("devloop.forge_validators")
local decimal_checksum = strings.decimal_checksum

function BranchTrain.install(M, shared)
  local require_safe_branch = shared.require_safe_branch
  local require_safe_sha = shared.require_safe_sha
  local require_safe_repo = shared.require_safe_repo
  local require_sync_result = shared.require_sync_result
  local runtime_root_path = shared.runtime_root_path

  function M.branch_sync_lock_key(repo, upstream, integration)
    local key = "github-devloop/branch-sync/"
      .. base_ids.safe_repo(require_safe_repo(repo))
      .. "/"
      .. require_safe_branch("upstream branch", upstream)
      .. "/"
      .. require_safe_branch("integration branch", integration)
    if not strings.is_path_safe_key(key, M._max_key_len) then
      error("github-devloop: invalid branch sync lock key")
    end
    return key
  end

  function M.rollup_lock_key(repo, upstream, integration)
    local key = "github-devloop/rollup/"
      .. base_ids.safe_repo(require_safe_repo(repo))
      .. "/"
      .. require_safe_branch("upstream branch", upstream)
      .. "/"
      .. require_safe_branch("integration branch", integration)
    if not strings.is_path_safe_key(key, M._max_key_len) then
      error("github-devloop: invalid rollup lock key")
    end
    return key
  end

  function M.rollup_source_ref(repo, pr_number)
    return {
      kind = "external",
      ref = require_safe_repo(repo) .. "#pr/" .. tostring(pr_number),
    }
  end

  function M.rollup_dedup_key(repo, upstream, integration, pr_number, head_sha)
    return base_ids.dedup_key({
      "rollup",
      require_safe_repo(repo),
      require_safe_branch("upstream branch", upstream),
      require_safe_branch("integration branch", integration),
      tostring(pr_number),
      require_safe_sha("head sha", head_sha),
    })
  end

  function M.rollup_ready_payload(repo, upstream, integration, pr_number, head_sha)
    return {
      schema = "github-devloop.v1",
      repo = require_safe_repo(repo),
      pr_number = pr_number,
      upstream_branch = require_safe_branch("upstream branch", upstream),
      integration_branch = require_safe_branch("integration branch", integration),
      head_sha = require_safe_sha("head sha", head_sha),
      dedup_key = M.rollup_dedup_key(repo, upstream, integration, pr_number, head_sha),
      source_ref = M.rollup_source_ref(repo, pr_number),
    }
  end

  function M.validate_rollup_ready(payload)
    if type(payload) ~= "table" then
      return false, "payload-not-table"
    end
    if payload.schema ~= "github-devloop.v1" then
      return false, "schema"
    end
    if not forge_validators.is_git_ref_safe(payload.upstream_branch) then
      return false, "upstream-branch"
    end
    if not forge_validators.is_git_ref_safe(payload.integration_branch) then
      return false, "integration-branch"
    end
    if not forge_validators.is_git_sha(payload.head_sha) then
      return false, "head-sha"
    end
    if type(payload.source_ref) ~= "table" then
      return false, "source-ref"
    end
    if payload.source_ref.kind ~= "external" then
      return false, "source-ref-kind"
    end
    local repo_ok, repo_err = pcall(function()
      require_safe_repo(payload.repo)
    end)
    if not repo_ok then
      return false, "repo-validation: " .. tostring(repo_err)
    end
    if tostring(payload.source_ref.ref or "") ~= M.rollup_source_ref(payload.repo, payload.pr_number).ref then
      return false, "source-ref-ref"
    end
    if tostring(payload.dedup_key or "") ~= M.rollup_dedup_key(
      payload.repo,
      payload.upstream_branch,
      payload.integration_branch,
      payload.pr_number,
      payload.head_sha
    ) then
      return false, "dedup-key"
    end
    return true, "ok"
  end

  function M.is_supported_rollup_ready(payload)
    local ok = M.validate_rollup_ready(payload)
    return ok == true
  end

  function M.branch_sync_source_ref(repo, upstream, integration)
    return {
      kind = "external",
      ref = require_safe_repo(repo)
        .. "#branch-sync/"
        .. require_safe_branch("upstream branch", upstream)
        .. "/"
        .. require_safe_branch("integration branch", integration),
    }
  end

  function M.branch_sync_dedup_key(repo, upstream, integration, upstream_sha)
    return base_ids.dedup_key({
      "branch-sync",
      require_safe_repo(repo),
      require_safe_branch("upstream branch", upstream),
      require_safe_branch("integration branch", integration),
      require_safe_sha("upstream sha", upstream_sha),
    })
  end

  function M.sync_commit_marker(repo, upstream, integration, upstream_sha, integration_parent, result)
    return '<!-- fkst:github-devloop:sync:v1 repo="' .. require_safe_repo(repo)
      .. '" upstream="' .. require_safe_branch("upstream branch", upstream)
      .. '" integration="' .. require_safe_branch("integration branch", integration)
      .. '" upstream_sha="' .. require_safe_sha("upstream sha", upstream_sha)
      .. '" integration_parent="' .. require_safe_sha("integration parent", integration_parent)
      .. '" result="' .. require_sync_result(result)
      .. '" -->'
  end

  function M.sync_commit_message(repo, upstream, integration, upstream_sha, integration_parent, result)
    return "Sync " .. require_safe_branch("upstream branch", upstream)
      .. " into " .. require_safe_branch("integration branch", integration)
      .. "\n\n"
      .. M.sync_commit_marker(repo, upstream, integration, upstream_sha, integration_parent, result)
  end

  function M.branch_sync_worktree_path(runtime_root, repo, upstream, integration, integration_sha)
    local slug = strings.sanitize_key(
      require_safe_repo(repo)
        .. "-"
        .. require_safe_branch("upstream branch", upstream)
        .. "-"
        .. require_safe_branch("integration branch", integration),
      false
    ):gsub("/", "-")
    slug = slug:gsub("%-+", "-"):gsub("^%-+", ""):gsub("%-+$", "")
    if slug == "" then
      slug = "branch-sync"
    end
    if #slug > 90 then
      slug = slug:sub(1, 90):gsub("%-+$", "")
    end
    local suffix = decimal_checksum(
      require_safe_repo(repo)
        .. "#"
        .. require_safe_branch("upstream branch", upstream)
        .. "#"
        .. require_safe_branch("integration branch", integration)
        .. "#"
        .. require_safe_sha("integration sha", integration_sha)
    )
    return runtime_root_path(runtime_root) .. "/worktrees/sync-" .. slug .. "-" .. suffix
  end

  function M.branch_sync_message_file(runtime_root, repo, upstream, integration, upstream_sha, integration_sha)
    local suffix = decimal_checksum(
      require_safe_repo(repo)
        .. "#"
        .. require_safe_branch("upstream branch", upstream)
        .. "#"
        .. require_safe_branch("integration branch", integration)
        .. "#"
        .. require_safe_sha("upstream sha", upstream_sha)
        .. "#"
        .. require_safe_sha("integration sha", integration_sha)
    )
    runtime_root_path(runtime_root)
    return "/tmp/fkst-github-devloop-sync-message-" .. suffix .. ".txt"
  end

  function M.is_supported_sync_conflict(payload)
    if type(payload) ~= "table"
      or payload.schema ~= "github-devloop.v1"
      or not forge_validators.is_git_ref_safe(payload.upstream_branch)
      or not forge_validators.is_git_ref_safe(payload.integration_branch)
      or not forge_validators.is_git_sha(payload.upstream_sha)
      or not forge_validators.is_git_sha(payload.integration_sha)
      or type(payload.source_ref) ~= "table"
      or payload.source_ref.kind ~= "external" then
      return false
    end

    local ok = pcall(function()
      require_safe_repo(payload.repo)
    end)
    if not ok then
      return false
    end

    local expected_ref = M.branch_sync_source_ref(payload.repo, payload.upstream_branch, payload.integration_branch)
    local expected_branch_sync = tostring(payload.source_ref.ref or "") == expected_ref.ref
      and tostring(payload.dedup_key or "") == M.branch_sync_dedup_key(
      payload.repo,
      payload.upstream_branch,
      payload.integration_branch,
      payload.upstream_sha
    )
    if expected_branch_sync then
      return true
    end

    local expected_pr_freshness = tostring(payload.source_ref.ref or ""):match("^" .. require_safe_repo(payload.repo):gsub("([^%w])", "%%%1") .. "#pr/%d+$") ~= nil
      and tostring(payload.dedup_key or "") == M.pr_freshness_dedup_key(
        payload.repo,
        payload.integration_branch,
        payload.upstream_sha
      )
    return expected_pr_freshness
  end
end

return BranchTrain
