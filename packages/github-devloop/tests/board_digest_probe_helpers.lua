local core = require("core")

local M = {}

M.spec = {
  consumes = { "board_digest_probe" },
  produces = { "board_digest_result" },
}

function M.run(payload)
  if payload.mode == "block" then
    return {
      body = core.board_digest_block(payload.repo, payload.tick),
    }
  end

  if payload.mode == "append" then
    return {
      proposal = core.append_board_digest_to_proposal(payload.proposal, payload.repo, payload.tick),
    }
  end

  if payload.mode == "board_loop" then
    return {
      proposal = core.build_board_loop_proposal(
        payload.repo,
        payload.issue_number,
        payload.current,
        payload.source_ref,
        payload.n,
        payload.converge,
        payload.tick
      ),
    }
  end

  if payload.mode == "board_review" then
    return {
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
    }
  end

  if payload.mode == "board_review_loop" then
    return {
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
    }
  end

  error("github-devloop test probe: unknown mode")
end

function pipeline(event)
  local payload = event.payload or {}
  raise("board_digest_result", M.run(payload))
end

M.pipeline = pipeline

return M
