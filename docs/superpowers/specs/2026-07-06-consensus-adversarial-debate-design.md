# Consensus Adversarial Debate — Design Spec

Date: 2026-07-06
Status: synthesis complete; pending user review
Companion: `2026-07-06-consensus-adversarial-debate-record.md` (full verbatim debate record — the 留痕 this spec's conclusions are audited against)

## 1. Purpose and provenance of this spec

User vision: the consensus engine should behave like "three grandmaster philosophers debating each other, essentially discussing beauty from a whole-picture view, then finding the globally best solution." Beauty here is the repo's BEAUTY GATE sense: fidelity to the essence of the problem — the most beautiful solution is usually the correct one.

This spec was produced by the protocol it specifies, applied to itself: three seat-differentiated philosophers (teleology / parsimony / fidelity) wrote **blind** position papers against the coordinator's draft, then received each other's full papers and wrote **rebuttals** under an update-or-defend contract, and the coordinator **synthesized** the result. Blind verdicts: approve / abstain / abstain. Post-rebuttal verdicts: approve / approve / approve, with a strongly convergent amendment set. Every load-bearing repo citation below was verified by at least two independent seats. A ChatGPT Pro oracle request was dispatched as the advisory cross-family signal; it was still queued at synthesis time (recorded in provenance per §4.9's own rule: absence is fail-open and stamped).

Author conflict disclosure: the coordinator authored the draft under debate and acted as synthesis judge. Mitigations: the full debate record ships with this spec; every synthesis decision below names which seat's argument it adopts or rejects; the user is the final backstop.

## 2. Verified current-state defects (each fact seat-verified at file:line)

1. **Whole-picture judgment is forbidden by contract**: "State the reason that is specific to THIS angle; do not restate another angle's criterion" (`packages/consensus/prompts/angle.lua:4`). Partitioned-criteria voting — the seams between angles belong to nobody.
2. **One-paragraph argument budget**: the angle output contract is one verdict word + "one concise paragraph" (`prompts/angle.lua:12-13`).
3. **The integrating mind is starved and muzzled**: the meta-judge reads 600-char digests (`core.lua:20`, `core.lua:565-589`) — each emitted twice (Reply/Digest duplication, `core/prompt_rendering.lua:55-58`, with `digest = reply` whenever parse succeeded, `core.lua:574`) — and must answer in exactly one line (`prompts/meta_judge.lua:15`). Full angle stdout exists in memory (`departments/decide/main.lua:82`) and is discarded.
4. **Convergence re-rounds are amnesiac**: a re-round worker sees exactly one added line, the `convergence_question` (`core/prompt_rendering.lua:88-91`). `prior_round_digests` is built upstream (`libraries/devloop/payloads/builders.lua:364`), validated by consensus (`core.lua:234-247, 304`), and rendered into **no prompt anywhere** — dead cargo, inspected at every station and unloaded at none.
5. **The thin wire is deliberate**: `packages/github-devloop/tests/core_flow_test.lua:786-792` documents cross-round peer-invisibility as design intent ("only verdict + short-reply digests, never prior peer full text"). The current architecture is Delphi-by-design: blind re-rounds anchored (at most) on majority digests, with no author present to defend and no confrontation. Delphi's documented pathology — the lone dissenter who saw the flaw gets washed out by the majority anchor — points in the false-approve direction.
6. **The round cap is an amnesia-era constant**: `max_converge_rounds() = 8` (`libraries/devloop/config.lua:135-137`); the historical round-33 livelock (#586) shows what amnesiac re-rolls do.
7. Ambient exhibit of ceremony accreting unread: `published_seam = { "proposal" }` declared twice in one spec table (`departments/decide/main.lua:6,8`).

## 3. Essence (adjudicated)

The consensus engine exists to (a) prevent **false approves** — plausible-but-wrong outcomes that produce no error fact, no stall, and green CI, invisible to both the safety and liveness nets (the competence axis's deepest blind spot), and (b) find **globally best framings** in design search — under correlated same-family sensors, minutes-per-spawn cost, an at-least-once idempotent pipeline, the ~64KiB content-not-in-payload constitution, and adversarially untrusted input.

Adjudicated first principles (each survived hostile rebuttal):

- **The unit of deliberative value is the argument confronted, not the verdict counted** (parsimony) — but the *fast path's* value comes from mechanism structure, not argument: uncoordinated blind agreement among differently-seeded judges is the one cheap signal that cannot be manufactured (teleology's derivation, conceded by all).
- **Confrontation breaks correlated error if and only if its currency is verifiable against ground truth external to both narrators** (fidelity's root correction, adopted): a narrative attacking a narrative is unsigned conditioning and may homogenize (cascade / correlated-hallucination risk, per the constitution's own incident record). Citations checkable now and outcomes checkable later are the two honest currencies.
- **Beauty is the shared judgment standard because the target failure class is ugly in enumerable ways** (six smells) — and beauty is never self-graded; it is adversarially applied (BEAUTY GATE's own placement instruction).
- **The engine submits to the instrument it applies**: every verdict is a wager recorded with its provenance, joinable against outcome ground truth (fidelity's falsifiability spine, adopted as non-negotiable).

## 4. Protocol

### 4.1 Seats

Three philosopher seats, each **whole-picture** (the `angle.lua:4` restriction is deleted), each carrying the full six-smell rubric, each seeded with one demonstrably disjoint lens:

- **teleology** — skipped-purpose, missing-inevitability: what is this for; is the form forced by the purpose?
- **parsimony** — magic numbers, symptom branches: delete until nothing is left to delete; every element proves its right to exist.
- **fidelity** — proxy-over-truth, narrative-over-verification: does it measure the real thing; is every premise verified at its source?

Seat count footing (adjudicated): primary — 3 is the minimal odd count giving pairwise confrontation plus a tiebreak (two deadlock, one is capturable); secondary coverage heuristic — the six smells factor into three families. The factorization alone was judged insufficient footing (parsimony's own confessed weakness, asserted as theorem by teleology and demoted in rebuttal).

Gate (PR-review) mode retains the **high-risk/security seat** outside the beauty debate: prompt injection is a different threat model, not an aesthetic stance to be argued out of. Its output is visible to the synthesis judge; its rejects route through GAP as today.

Seat-level disagreement rate becomes a provenance/ledger column so choir-collapse (three seats sharing one prior despite disjoint seeds) is detectable rather than assumed away.

### 4.2 Phase B — blind positions (sacred; unchanged cost)

Peer-invisible, parallel, full source content via the existing `content_fetch` manifest (blind-but-fully-informed: evidence partition was adjudicated OUT — it reproduces `angle.lua:4` at the input layer and manufactures disagreement by starvation, failing fidelity's own Goodhart test and the constitution's full-content doctrine).

Per-seat contract, converge (design-search) mode:
1. `ESSENCE` — independently derived problem essence, written BEFORE engaging the proposal's own story (mandated prose section; see sentinel diet §4.6).
2. `IDEAL` — sketch of the most faithful solution, unconstrained by the proposal (prose).
3. Six-smell comparison of the proposal against that ideal (prose; checkable smells cited to file:line where they exist).
4. `⟦FKST:VERDICT⟧` (sentinel) + `⟦FKST:REPLY⟧` (sentinel) + `WEAKEST` — self-named weakest assumption (mandated prose; the rebuttal handle).

Gate mode calibration (adjudicated, all three seats): the ideal sketch is **context only, never a reject ground** — good-enough-and-clean is approvable; every blocking claim must name an evidenced smell citing into the diff, carried by `⟦FKST:GAP⟧` as today. A gate that rejects the good for not being the best manufactures fix-loop churn.

### 4.3 Escalation rule (one rule, both modes)

**The debate replaces the meta-judge.** Escalation triggers exactly where today's deterministic aggregate yields no decision (`core.lua:504-563`): converge mode — no blind unanimity; gate mode — no decisive aggregate (the all-abstain/comment middle, or reject-without-parseable-gap). The deterministic aggregate is never overridden by debate; the blind unanimity fast path is unchanged at 3 spawns.

This is the synthesis of the gate-mode split (teleology: no debate in gate — the fix loop is the gate's native confrontation and the author its missing party; parsimony: debate only for the ambiguous middle; fidelity: debate on verdict split): the debate runs precisely where a judge already runs today, and nowhere else. Evidenced rejects keep flowing to the fix loop — the gate's native confrontation — untouched.

### 4.4 Phase R — rebuttal (only on escalation)

Each seat receives: its own P1 paper (position lock-in) plus both peers' **full, neutralizer-wrapped** P1 papers. Contract:

- Attack the **root** — the peer's essence derivation — before the leaves.
- **Fail-closed search, never mandated conclusion**: "name the weakest premise, or state with evidence why none is found." (All three seats independently refuted the forced-attack device in the blind round — Goodhart at the prompt layer; Nemeth: assigned devil's advocates produce less belief revision and can inoculate the majority. A forced move carries zero bits.)
- **Fact-citation currency**: attacks name checkable facts with provenance resolvable within in-invocation materials only — the context-manifest files (already fetched), the proposal body, the peer transcripts. No new fetches, no network, matching the existing execution boundary (`prompts/angle.lua:6-9`). Claims checkable by derivation (e.g. inevitability: does the purpose force this form?) are audited as derivations; claims checkable by neither are flagged unverified narrative.
- `⟦FKST:STANCE⟧ update|defend` (sentinel, fail-closed — it routes) — an update must name the specific peer claim that moved it; a defend must name the evidence that defeats the attack. Updated `⟦FKST:VERDICT⟧` + `⟦FKST:REPLY⟧`.

After Phase R: re-check unanimity on updated verdicts. Unanimous ⇒ reached with provenance `post-rebuttal-unanimity`. STANCE flips are routing signals only — never debate-quality telemetry; only citation-verified moves count as evidence the debate bit (fidelity's rider, adopted).

### 4.5 Phase S — synthesis (only if Phase R did not conclude)

One judge, full P1+P2 transcript in-memory (the payload constitution binds payloads, not invocations). Duties:

1. **Verify the load-bearing citations** — exactly those named as movers in STANCE updates plus those its own decision rests on, against the shared context manifest, inside its own spawn. **Unverified ⇒ weight zero** (fail-soft with evidential consequence; never a fail-closed abort — an essay-shape defect costs one claim its weight, not seven spawns). No verification quotas or caps: the judge may weigh only what it verified; self-bounding by decision-relevance.
2. **Adjudicate and, where positions are compatible, graft** a globally-best framing with per-element attribution to the seat whose argument supplied it.
3. **Author the findings-of-fact record** (§4.7).
4. Output exactly one of:
   - `reached:approve <framing>` / `reached:reject <framing>` (gate mode) — approve semantics: no ugliness survived adversarial scrutiny at the evidence level;
   - `converge:<named essence-level disagreement> + <the concrete evidence that would resolve it>` (§4.8).

The `⟦FKST:PLAN⟧` outcome is **deleted** — a hedge state made unrepresentable-by-need by a synthesis judge empowered to graft the merged framing and approve it. Outcomes collapse to `reached | converge`. A synthesis output that fails to parse is retried **at one-spawn width** — never by re-running the debate (deleting today's `default_narrowed_question` fallback economics, `core.lua:650-661`).

### 4.6 Sentinel diet (adjudicated, unanimous)

Fail-closed sentinel parsing is earned by a **machine branch**, nothing else: `VERDICT` (routes), `STANCE` (routes the post-R unanimity re-check), `GAP` (routes gate rejects), `REPLY` (anchors the unique-adjacent-pair discipline). `ESSENCE` / `IDEAL` / `WEAKEST` / findings-record interiors are **mandated prose sections** — wired to their real reader (the adversary seats and the judge, who read full text), peer-attackable, parser-invisible. Rationale: under the parser's cost model (`core.lua:396-474`, any malformed pair ⇒ nil ⇒ whole spawn wasted), every ceremonial sentinel converts an essay-shape defect into minutes of burned frontier compute. Fidelity's "wired or deleted" dictum and the diet are one principle: the disease is fields with no reader (`prior_round_digests`), not prose with an epistemic reader.

### 4.7 Inter-round memory: the findings-of-fact record (THE one canonical memory)

Synthesis authors a structured record:

```
settled: <X>, by refutation of <Y> ⟨verified citation⟩
settled-by-agreement (unverified): <X'>
open: <Z>
```

- Written to the existing converge marker surface (trusted-bot comment), bounds-validated at write; rehydrated by any continuation round via the existing `content_fetch` mechanism; **neutralizer-wrapped on every rehydration** (it is model-authored text).
- **Provenance-carrying entries are mandatory** (fidelity's anti-laundering rider): an adjudicated distillation is model-authored narrative that becomes the next round's premise — entries must carry the citation that settled them or be explicitly labeled settled-by-agreement/unverified, so continuation workers and human auditors can distinguish facts-of-record from stamped narrative.
- It **replaces** the digest deliberation channel: `prior_round_digests` (field, validator, `max_prior_round_digests`, builder write) and the Reply/Digest duplication are deleted; `narrowed_question` survives only as a one-line label/pointer, not a deliberative channel. One memory, not three (parsimony's replacement guard).
- Nothing deliberative crosses a reliable payload — the record rides the durable marker surface, per the content-not-in-payload constitution.

### 4.8 Outer loop: resolvability gate (budget derived, not inherited)

A `converge` output must name **both** the essence-level disagreement **and** the concrete evidence that would resolve it:

- Evidence unnameable ⇒ converge is **terminal** ⇒ reconcile to `blocked` with WHY (essence-stall). This is honest termination, not failure.
- Evidence named ⇒ exactly **one** evidence-continuation debate runs for this proposal version, rehydrating the findings record. If the named evidence did not resolve the disagreement, that *is* essence-stall by definition ⇒ terminal.
- New external facts arrive ⇒ new proposal version (existing dedup_key versioning) ⇒ fresh debate. The continuation budget is per proposal version.

The inherited cap of 8 (`config.lua:135-137`) is **deleted** — it was priced for 4-spawn amnesiac re-rolls (worst case 32 spawns); carried into 7-spawn debates it silently prices a 56-spawn worst case nobody chose. Derived worst case per proposal version: 7 (debate) + 7 (one continuation) = 14 spawns, vs today's up-to-32. True-stall reconcile and all saga liveness rows are unchanged in shape.

### 4.9 Cross-family oracle: advisory-with-measured-consequence

Adjudicated against both extremes (hard verdict seat: violates guaranteed-termination — a browser-pool, quota-bound, 403-prone boundary resource cannot hold a veto over pipeline liveness, and the constitution's own operational text keeps verdict routing on codex envelopes; silent advisory: dead cargo by the `prior_round_digests` diagnosis). The adopted form:

- At escalation time (start of Phase R — the earliest moment a debate is known to exist; unanimous proposals never consume oracle quota), fire `nyxid oracle ask --no-wait` with the proposal + manifest pointers. The oracle races the rebuttal phase.
- At Phase S, if a result exists: the judge must **address (not obey)** each oracle-named smell in the findings — attribute-and-answer. Absent ⇒ proceed fail-open.
- Presence/absence and address/dismiss are **stamped into verdict provenance**, so the ledger continuously measures whether oracle-consulted debates have lower false-approve rates. Promotion of the oracle to a hard verdict lane is an **evidence-gated upgrade** contingent on that measurement — never a founding assumption.

### 4.10 Provenance stamping (the falsifiability spine; non-negotiable)

Every `consensus_reached` payload and its durable marker carry:

- `verdict_path`: `blind-unanimity | post-rebuttal-unanimity | synthesis`
- dissent-and-fold trace (who dissented in P1, who updated in P2, movers named)
- `verified_moves`: count of citation-verified claims that changed a verdict or entered a graft (zero ⇒ the transcript is theater, and the record says so)
- `oracle_consulted` / `oracle_addressed`
- memory channel used on continuations (`memory=findings` | `memory=digest` during calibration)
- seat verdict vectors per phase (P1 verdicts, P2 verdicts) — the choir-detection column: persistent zero P1 disagreement across proposals despite disjoint seeds indicates seat collapse

These are small control fields (payload-legal). They join against the false-consensus / AVM ledger (#879 track) — this spec defines the join point, not the ledger. Rationale: the design's central empirical claim — post-rebuttal unanimity is a stronger signal than blind unanimity — must be falsifiable in production; the blind fast path provides a continuous baseline. Selection differences (controversial vs not) make this per-path calibration, not randomized A/B; per-path outcome rates over time are sufficient to falsify "post-rebuttal unanimity is reliable," which is what matters.

### 4.11 Honest limits (adjudicated demotions — kept in the spec on purpose)

- **Intra-family rebuttal breaks the within-capability subclass of correlated error only** (parsimony's demotion, upheld): errors the whole model family cannot see once pointed at remain covered only by the oracle advisory signal, the human backstop, and the outcome ledger. The spec does not claim otherwise.
- **Agreement-after-mutual-reading carries less evidential weight than blind agreement** (fidelity, upheld): reading peers is conditioning; the citation currency is what signs it positive. Hence provenance separates the paths forever.
- **The findings record is model-authored narrative** — mitigated by provenance-carrying entries, not cured. If ledger data shows continuation rounds degrade toward re-rolls, teleology's own WEAKEST predicts the failure and the record format is the first suspect.
- The claim "three disjoint-seeded seats decorrelate better than today's four disjoint biases" is **unverified** (parsimony, upheld) — the seat-disagreement provenance column exists to test it, not to assume it.

## 5. Security

- Peer P1/P2 texts, oracle text, and the rehydrated findings record pass through the existing neutralizer (`core/prompt_rendering.lua:3-31`, extended to the new sentinels) at **every** embedding: all of it may second-hand-echo injected instructions from the untrusted proposal body.
- Verdict/stance/gap extraction stays fail-closed unique-adjacent-pair on bounded document tails.
- Execution boundary unchanged: judgment scratch directories, no clone/fetch/branch, content only via manifest (`prompts/angle.lua:6-9`); rebuttal citations resolvable from in-invocation materials only — no new fetches.
- High-risk seat remains outside the beauty debate (different threat model), visible to synthesis, GAP-routed.

## 6. Budgets, liveness, topology

- **No new states, queues, or marker kinds.** The debate lives inside one `decide` invocation (proven shape: multi-spawn + `await_all` + sync judge; long sync codex is an existing pattern). Event topology, saga rows, and liveness contracts are unchanged in shape.
- Crash-only, **no per-phase checkpoints**: a crash mid-debate re-runs the whole debate; dedup stays at the decision boundary (`decide/main.lua:164-173`). The retry quantum grows from 4 to 7 spawns; accepting that is cheaper and more honest than per-phase durable state (a second truth source).
- Bounded by construction: 3 (blind) [+ 3 (rebuttal) + 1 (synthesis) on escalation] spawns per debate; ≤ 2 debates per proposal version (§4.8); oracle is advisory and non-blocking.
- Cost honesty (parsimony's demand): "7 vs 32" is a worst-case bound comparison, not a measurement. The calibration experiment (§7) and slice-1 provenance baseline produce the actual distribution; expected outer recurrence under the resolvability gate is near zero.

## 7. Calibration experiment (parallel, never gating)

The 0-spawn hypothesis ("the pathology is memory, not debate") was adjudicated: rendering `prior_round_digests` is itself a **low-bandwidth debate** (cross-round, conditioned, 600-char, no author present) — a confounded experiment, not a control; and it can only measure the convergence axis, not the false-approve axis that dominates by the asymmetry premise. The sequencing demand was withdrawn by its own author. What survives:

- Wire prior-round findings/digests into the blind re-round prompts wherever blind re-rounds still exist during migration (and gate-mode re-rounds), stamped `memory=digest` vs `memory=findings` in provenance — zero extra spawns.
- Use the measured round-count distribution to price the cost narrative and validate the continuation budget.
- Retire the wire together with the old architecture.

## 8. Deletion manifest (each item dies in the same PR as its replacement)

A spec that does not name its deletions will be implemented as an addition (parsimony; the standing proof is `prior_round_digests` itself).

| Deletion | Where | Replaced by |
|---|---|---|
| `prior_round_digests` field + validator + `max_prior_round_digests` + builder write | `core.lua:234-247,304`, `core.lua:21`, `builders.lua:364` | findings-of-fact record (§4.7) |
| Reply/Digest duplication | `prompt_rendering.lua:55-58`, `core.lua:572-577` | full transcript in-memory; digests only as outer telemetry |
| `⟦FKST:PLAN⟧` outcome | `prompts/meta_judge.lua:12-14`, `core.lua:627-632` | grafting synthesis (§4.5) |
| `default_narrowed_question` as debate-wide fallback | `core.lua:650-661` | one-spawn-width synthesis retry |
| meta-judge prompt contract (one-line budget) | `prompts/meta_judge.lua` | synthesis contract (§4.5) — replaced, not paralleled |
| angle-restriction line (anti-whole-picture) | `prompts/angle.lua:4` | whole-picture seats (§4.1) |
| inherited round cap 8 | `config.lua:135-137` | resolvability gate (§4.8) |
| duplicate `published_seam` key | `decide/main.lua:6,8` | single declaration |

## 9. Implementation slices (each independently mergeable, each carrying its own deletions)

1. **Provenance stamping of today's paths** (`blind-unanimity` / `synthesis`≈meta-judge, oracle fields absent): baseline outcome data starts accruing before the debate lands; smallest slice, ships first.
2. **Phase B contract upgrade**: philosopher seats + disjoint lens seeds, whole-picture (delete `angle.lua:4` restriction), ESSENCE/IDEAL/WEAKEST prose sections, gate calibration clause (ideal-as-context, cited GAP rejects).
3. **Debate core**: rebuttal phase + synthesis judge replacing the meta-judge; STANCE sentinel + parser; citation discipline (unverified ⇒ unweighed); neutralizer extension; PLAN + `default_narrowed_question` + meta-judge deletions; one-spawn synthesis retry.
4. **Outer loop**: findings-of-fact record on the converge marker + rehydration; resolvability gate; budget re-derivation (delete cap 8); `prior_round_digests` deletion; calibration wire (in and, later, out).

File-size discipline: `core.lua` is at 791 lines; debate-phase logic goes into department-local modules (`departments/decide/*.lua`) and prompt modules, splitting at the 900-line soft threshold — never packed into one file to "stay small in file count."

## 10. Testing

- Parser: STANCE fail-closed forms; injection — peer transcripts echoing sentinel-lookalikes at P2/P3 embeddings; unique-adjacent-pair on long documents; unverified-citation weight-zero behavior.
- `run_department` integration (mock codex): blind-unanimity fast path spawns exactly 3 and skips debate; escalation spawns rebuttal + synthesis; a STANCE update flips the post-R unanimity re-check; gate mode — evidenced reject bypasses debate to the fix loop, ambiguous middle escalates; provenance fields present and correct per path; findings record written/rehydrated/neutralized; continuation budget: second converge on same proposal version terminates to reconcile.
- Production-fidelity harness rules apply: namespaced queue delivery, no `now()` time bombs, fail-closed unmocked commands.
- Conformance: no new states — existing liveness rows must pass unchanged; G-CONTENT-TRUNCATION: the findings record is adjudicated-bounded, not mechanically truncated (no new `max_*_len` into payloads or prompts).

## 11. Debate record summary

| Seat | Blind verdict | Post-rebuttal | Decisive contributions (adopted) |
|---|---|---|---|
| teleology | approve | approve | Delphi-by-design finding (`core_flow_test.lua:786-792`); findings-of-fact memory; resolvability gate; gate-mode churn calibration; oracle advisory-but-answered; fast-path derivation from mechanism structure |
| parsimony | abstain | approve | deletion manifest doctrine; sentinel diet; one-canonical-memory guard; seat-count re-footing; unverified⇒unweighed citation bound; escalation-where-judge-already-runs (gate middle); calibration-parallel-not-gating |
| fidelity | abstain | approve | provenance stamping (falsifiability spine); fact-citation currency + cascade correction to Corollary 2; anti-laundering findings rider; oracle measured-consequence form; honest-limits demotions; blind-round sanctity argument |

Cross-cutting: all three independently refuted the forced-attack device in the blind round (Goodhart / Nemeth / zero-bits — three routes, one conclusion), which is itself the strongest in-process evidence that seat-differentiated blind judgment decorrelates on at least some errors. Full verbatim record: companion file.

⟦AI:FKST⟧
