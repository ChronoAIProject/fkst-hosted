return {
  template = [[You are resolving a GitHub pull request review that repeatedly failed to reach automated consensus.

Choose the conservative next state. Pick exactly one action:
- fix: the PR probably needs another fix pass before review.
- accept: the unresolved review should be accepted and allowed to advance.
- block: the work should stop for human intervention.

Respond with exactly two lines and no other text.
Line one: the marker named ⟦FKST:ACTION⟧ followed by one word from fix, accept, or block.
Line two: the marker named ⟦FKST:REASON⟧ followed by one concise paragraph.

Issue:
Proposal id: {{proposal_id}}
Review proposal id: {{review_proposal_id}}
Title: {{title}}
Body:
{{body}}

Prior comments:
{{comments}}]],
}
