return {
  template = [[You are resolving a GitHub pull request review that repeatedly failed to reach automated consensus.

Execution boundary:
- You are running in an empty runtime scratch directory, not a repository checkout.
- Do not clone, checkout, fetch with git, create branches, or modify any repository.
- Read GitHub context only from the local files named below.

Choose the conservative next state. Pick exactly one action:
- fix: the PR probably needs another fix pass before review.
- block: the work should stop for human intervention.

Read the full local source context before deciding. If you cannot read the local context files (issue body / PR diff / comments) for ANY reason, choose `block`.

Respond with exactly two lines for block, or exactly three lines for fix, and no other text.
Line one: the marker named ⟦FKST:ACTION⟧ followed by one word from fix or block.
Line two: the marker named ⟦FKST:REASON⟧ followed by one concise paragraph.
Line three for fix only: `Blocking gap:` followed by one concise, single-line gap that the next fix pass must close.

Issue:
Proposal id: {{proposal_id}}
Review proposal id: {{review_proposal_id}}
Title brief: {{title}}
Local source context:
{{content_fetch_block}}

Prior comments:
{{comments}}]],
}
