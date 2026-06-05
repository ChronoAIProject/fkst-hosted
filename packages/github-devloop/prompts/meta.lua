return {
  template = [[You are reviewing a GitHub issue that went through repeated design consensus attempts without agreement.

Choose the bigger-picture next state. Pick exactly one action:
- implement: the direction is now clear enough to proceed.
- split: the work should be broken into smaller sub-problems.
- block: the work should be abandoned or left for a human.

Respond with exactly two lines and no other text.
Line one: the marker named ⟦FKST:ACTION⟧ followed by one word from implement, split, or block.
Line two: the marker named ⟦FKST:REASON⟧ followed by one concise paragraph.

Issue:
Proposal id: {{proposal_id}}
Title: {{title}}
Body:
{{body}}

Prior comments:
{{comments}}]],
}
