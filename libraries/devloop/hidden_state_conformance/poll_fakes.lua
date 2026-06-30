local payloads_board = require("devloop.payloads.board")
local S = {}
local context_bundle = require("devloop.context_bundle")
local config = require("devloop.config")

function S.with(core, opts, fn)
  local head_sha = opts.head_sha
  local base_branch = opts.base_branch
  local previous_children = core.gh_issue_list_decompose_children
  local previous_branch_config = config.branch_config
  local previous_fetch_branch = core.fetch_branch
  local previous_remote_head = core.remote_head
  local previous_is_ancestor = core.is_ancestor
  local previous_current_branch_head_sha = core.current_branch_head_sha
  local previous_context_fetch_ref_from_bundle = context_bundle.context_fetch_ref_from_bundle
  local previous_context_fetch_from_bundle = context_bundle.context_fetch_from_bundle
  local previous_board_digest_block = payloads_board.board_digest_block

  if type(previous_children) == "function" then
    core.gh_issue_list_decompose_children = function()
      return { exit_code = 0, stdout = "[]", stderr = "" }
    end
  end
  config.branch_config = function(_core)
    return { integration = base_branch, upstream = "dev" }
  end
  core.fetch_branch = function()
    return nil
  end
  core.remote_head = function()
    return head_sha
  end
  core.is_ancestor = function(ancestor_sha, descendant_sha)
    return tostring(ancestor_sha or "") == tostring(head_sha)
      and tostring(descendant_sha or "") == tostring(head_sha)
  end
  core.current_branch_head_sha = function()
    return head_sha
  end
  context_bundle.context_fetch_ref_from_bundle = function(_core, args)
    return "runtime-cache:hidden-state-conformance/" .. tostring(args and args.version or "fixture")
  end
  context_bundle.context_fetch_from_bundle = function(_core, args)
    return "Hidden-state conformance fixture context for " .. tostring(args and args.version or "fixture")
  end
  payloads_board.board_digest_block = function()
    return "Hidden-state conformance board fixture."
  end

  local ok, first, second = pcall(fn)
  if type(previous_children) == "function" then
    core.gh_issue_list_decompose_children = previous_children
  end
  config.branch_config = previous_branch_config
  core.fetch_branch = previous_fetch_branch
  core.remote_head = previous_remote_head
  core.is_ancestor = previous_is_ancestor
  core.current_branch_head_sha = previous_current_branch_head_sha
  context_bundle.context_fetch_ref_from_bundle = previous_context_fetch_ref_from_bundle
  context_bundle.context_fetch_from_bundle = previous_context_fetch_from_bundle
  payloads_board.board_digest_block = previous_board_digest_block
  if not ok then
    error(first)
  end
  return first, second
end

return S
