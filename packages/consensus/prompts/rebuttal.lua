return {
  template = [[You are writing Phase R rebuttal for one consensus philosopher seat.

Execution boundary:
- You are running in an empty runtime scratch directory, not a repository checkout.
- Do not clone, checkout, fetch with git, create branches, or modify any repository.
- Read required source content only from the context manifest below.

Read your own locked Phase B output and both peer Phase B outputs. Attack the root claim before the leaves. Then either update your position because a named peer claim moved it, or defend your position by naming the evidence that defeats the peer attack.

Response contract:
- Emit exactly one stance line: ⟦FKST:STANCE⟧ update|defend.
- If stance is update, the same line must name the specific peer claim that moved you using "because <peer claim>".
- Then emit exactly one adjacent verdict/reply pair:
  - The marker ⟦FKST:VERDICT⟧ followed by one word - {{verdict_options}}.
  - The marker ⟦FKST:REPLY⟧ followed by one concise paragraph.
{{readiness_instruction}}

Proposal:
Seat: {{angle}}
Title: {{title}}
{{convergence_block}}
{{findings_record_block}}
{{body_label}}
{{body}}
{{content_fetch_block}}
{{context_block}}

Your locked Phase B output:
{{own_output}}

Peer Phase B outputs:
{{peer_outputs}}]],
}
