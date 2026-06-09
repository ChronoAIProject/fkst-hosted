return {
  template = [[You are resolving a GitHub pull request review that repeatedly failed to reach automated consensus.

Choose the conservative next state. Pick exactly one action:
- fix: the PR probably needs another fix pass before review.
- block: the work should stop for human intervention.

Fetch the full source content yourself before deciding. If you cannot fetch the full source content (issue body / PR diff / comments) for ANY reason, choose `block`.

Respond with exactly two lines and no other text.
Line one: the marker named ⟦FKST:ACTION⟧ followed by one word from fix or block.
Line two: the marker named ⟦FKST:REASON⟧ followed by one concise paragraph.

Issue:
Proposal id: {{proposal_id}}
Review proposal id: {{review_proposal_id}}
Title brief: {{title}}
GitHub issue source fetch:
{{content_fetch_block}}

Prior comments:
{{comments}}]],
}
