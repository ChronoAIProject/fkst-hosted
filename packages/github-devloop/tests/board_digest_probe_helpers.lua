local core = require("core")

local M = {}

M.spec = {
  consumes = { "board_digest_probe" },
  produces = { "board_digest_result" },
}

local function raise_result(payload)
  raise("board_digest_result", payload)
end

function pipeline(event)
  local payload = event.payload or {}
  if payload.mode == "block" then
    raise_result({
      body = core.board_digest_block(payload.repo, payload.tick),
    })
    return
  end

  if payload.mode == "append" then
    raise_result({
      proposal = core.append_board_digest_to_proposal(payload.proposal, payload.repo, payload.tick),
    })
    return
  end

  if payload.mode == "board_loop" then
    raise_result({
      proposal = core.build_board_loop_proposal(
        payload.repo,
        payload.issue_number,
        payload.current,
        payload.source_ref,
        payload.n,
        payload.converge,
        payload.tick
      ),
    })
    return
  end

  if payload.mode == "board_review" then
    raise_result({
      proposal = core.build_board_pr_review_proposal(
        payload.repo,
        payload.issue_number,
        payload.pr_number,
        payload.version,
        payload.head_sha,
        payload.current,
        payload.source_ref,
        payload.tick
      ),
    })
    return
  end

  if payload.mode == "board_review_loop" then
    raise_result({
      proposal = core.build_board_pr_review_loop_proposal(
        payload.repo,
        payload.issue_number,
        payload.pr_number,
        payload.version,
        payload.head_sha,
        payload.current,
        payload.source_ref,
        payload.n,
        payload.converge,
        payload.tick
      ),
    })
    return
  end

  error("github-devloop test probe: unknown mode")
end

return M
