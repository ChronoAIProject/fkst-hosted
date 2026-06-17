local S = {}
local support = require("core.commands.support")
local validators = require("core.commands.validators")

function S.install(M)
  function M.gh_pr_list_board_digest(repo, timeout)
    return support.gh_result(function()
      return support.github().pr_list_board_digest(repo, timeout)
    end)
  end

  function M.gh_pr_list_freshness(repo, timeout)
    return support.gh_result(function()
      return support.github().pr_list(repo, timeout)
    end)
  end

  function M.gh_pr_list_merge_queue(repo, base, timeout)
    return support.gh_result(function()
      return support.github().pr_list_merge_queue(repo, validators.require_safe_branch(M, "merge queue base branch", base), timeout)
    end)
  end

  function M.gh_pr_view_origin(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(
        repo,
        pr_number,
        "headRefName,headRefOid,baseRefName,state,updatedAt,mergedAt,comments,labels,mergeable,mergeStateStatus",
        timeout
      )
    end)
  end

  function M.gh_pr_view_observe(repo, pr_number, timeout)
    return M.gh_pr_view_origin(repo, pr_number, timeout)
  end

  function M.gh_pr_view_fix(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,comments,headRepository,headRepositoryOwner,isCrossRepository", timeout)
    end)
  end

  function M.gh_pr_view_fix_precheck(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,updatedAt,comments,headRepository,headRepositoryOwner,isCrossRepository", timeout)
    end)
  end

  function M.gh_pr_view_merge(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,baseRefOid,state,updatedAt,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", timeout)
    end)
  end

  function M.gh_pr_view_freshness(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(repo, pr_number, "headRefName,headRefOid,baseRefName,state,updatedAt,isDraft,comments,labels,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", timeout)
    end)
  end

  function M.gh_pr_list_head_base(repo, head, base, timeout)
    return support.gh_result(function()
      return support.github().pr_list_head(
        repo,
        validators.require_safe_branch(M, "PR head branch", head),
        validators.require_safe_branch(M, "PR base branch", base),
        timeout
      )
    end)
  end

  function M.gh_pr_list_head(repo, head, timeout)
    return support.gh_result(function()
      return support.github().pr_list_head(repo, validators.require_safe_branch(M, "PR head branch", head), nil, timeout)
    end)
  end

  function M.gh_pr_create(repo, head, base, title, body_file, timeout)
    return support.gh_result(function()
      return support.github().pr_create(
        repo,
        validators.require_safe_branch(M, "PR head branch", head),
        validators.require_safe_branch(M, "PR base branch", base),
        title,
        body_file,
        timeout
      )
    end)
  end

  function M.gh_pr_merge(repo, pr_number, head_sha, timeout)
    if tostring(head_sha or "") == "" then
      error("github-devloop: invalid merge head sha")
    end
    return support.gh_result(function()
      return support.github().pr_merge(repo, pr_number, head_sha, timeout)
    end)
  end

  function M.gh_commit_check_runs(repo, head_sha, timeout)
    return support.gh_result(function()
      return support.github().api_get(repo, "commits/" .. validators.require_safe_sha(M, "commit check-runs head sha", head_sha) .. "/check-runs", timeout)
    end)
  end

  function M.gh_check_run_rerequest(repo, check_run_id, timeout)
    local id = tostring(check_run_id or "")
    if id == "" or id:find("[^0-9]") ~= nil then
      error("github-devloop: invalid check-run id")
    end
    return support.gh_result(function()
      return support.github().api_method("POST", "repos/" .. tostring(repo) .. "/check-runs/" .. id .. "/rerequest", nil, nil, nil, timeout)
    end)
  end

  function M.gh_pr_ready(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_ready(repo, pr_number, timeout)
    end)
  end

  function M.gh_issue_comment(repo, issue_number, body_file, timeout)
    return support.gh_result(function()
      return support.github().issue_comment(repo, issue_number, body_file, timeout)
    end)
  end

  function M.gh_pr_comment(repo, pr_number, body_file, timeout)
    return support.gh_result(function()
      return support.github().pr_comment(repo, pr_number, body_file, timeout)
    end)
  end

  function M.gh_pr_close(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_close(repo, pr_number, timeout)
    end)
  end

  function M.gh_pr_diff(repo, pr_number, timeout, run)
    return support.gh_result(function()
      return support.github(run).pr_diff(repo, pr_number, timeout)
    end)
  end

  function M.gh_pr_diff_name_only(repo, pr_number, timeout, run)
    return support.gh_result(function()
      return support.github(run).pr_diff_name_only(repo, pr_number, timeout)
    end)
  end

  function M.gh_pr_view_head(repo, pr_number, timeout)
    return support.gh_result(function()
      return support.github().pr_cli_view(repo, pr_number, "headRefName,baseRefName,state", timeout)
    end)
  end

  function M.gh_pr_view_context(repo, pr_number, timeout, run)
    return support.gh_result(function()
      return support.github(run).pr_cli_view(repo, pr_number, "title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels", timeout)
    end)
  end
end

return S
