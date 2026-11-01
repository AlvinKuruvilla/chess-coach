# Scoring the false-explanation experiment

The experiment in PROBLEM.md runs two ways of bridging Stockfish's evaluation
to a human explanation over the same games, and picks one. This is what it
counts.

Its first cut counted one thing: how often each arm produces a claim that is
false about the position. That cannot settle the fork. The detector arm asserts
only what it computed from the board, so modulo bugs its false-claim rate is
zero by construction, and an arm that emits nothing at all scores perfectly.
Ranking on false claims alone picks the detector arm before the experiment
runs.

The missing half is coverage: of what actually went wrong in a game, how much
did each arm find. What follows is a denominator for that, what it can and
cannot reach, and how the counts combine into a decision.

## Recall has to be per claim, not per position

The obvious construction — count the mistakes in a game, count how many each
arm explained — measures nothing. Which plies get explained is decided upstream
of the fork, by whatever detects critical moments; both arms are handed the
same list. Per-position recall is 100% for both, which is the mirror of the
false-claim-rate problem rather than a fix for it.

The arms differ in what they say about a position, not in which positions they
speak about. So the denominator lives inside a position: given this mistake,
what did a correct explanation have to contain.

## The denominator: what the refutation takes

For a mistake at ply n, Stockfish supplies the principal variation from the
position after the played move. Playing that variation forward and diffing the
board yields the concrete cost — which piece leaves the board, from which
square, or that the king does not survive. Those diffs are facts, derived from
the oracle, needing neither human annotation nor a model in the loop.

```text
  position after the mistake
            |
            +-- Stockfish depth D --> principal variation
                                          |
              play it forward, diff the board
                                          |
        +---------------------------------+------------------+
        |                                                    |
  tier 1: material transfer                        tier 2: the relation
  "the knight on f6 is lost"                "its defender was pinned to the king"
  fully mechanical, no ceiling              classifier-limited, see below
```

Measured 2026-08-29 on 150 mistakes sampled from the 400-999 band, selected by
Lichess `%eval` loss >= 200cp and analysed with local Stockfish at depth 18,
`Threads 1` (`tmp/mechanism_coverage.py`):

| outcome in the PV | share |
|---|---|
| material lost | 80.7% |
| mate net | 2.0% |
| capture, no net material change | 6.0% |
| no material change | 11.3% |

So **83% of mistakes in the target band have a mechanically-derivable
consequence**. The remaining 17% are mistakes whose cost is positional — a
square conceded, a structure ruined — where the diff comes back empty.

Those 17% are excluded from the recall denominator, not counted as misses.
Scoring an arm against an empty fact set would mark every explanation of a
positional mistake wrong regardless of merit. They stay in the false-claim
count, where whatever an arm says about them is still checked for truth.

## The relation is where the denominator gets a ceiling

Tier 1 alone is too coarse to discriminate. "You lose the knight on f6" scores
full material recall and is precisely the shallow-but-correct output the
experiment exists to weigh against something richer. The discriminating content
is the relation that made the loss available: undefended, defender pinned,
defender overloaded, forked, discovered attack, back rank.

Classifying that relation needs the same attack-map machinery as the detector
arm. This does not make the measurement circular — the classifier populates the
denominator for both arms, so it is oracle rather than contestant — but it does
mean **the tier-2 denominator inherits the detector arm's motif ceiling.** A
mistake whose mechanism falls outside the motif set is a position where tier-2
recall cannot be computed at all.

Rather than hide that, report it: positions where the relation is unclassified
are counted and excluded from tier-2 recall, and that count is part of the
result. It bounds how much of the question the measurement was able to reach.

## Where unbounded coverage would actually show up

The LLM arm's claimed advantage is positions no detector anticipates. That
advantage cannot appear in recall against a mechanically-built denominator,
because the denominator is exactly the set of things a detector can compute.
Measuring only recall would understate the LLM arm as badly as measuring only
false claims overstates it.

It appears instead as claims that are true, adjudicable, and absent from the
denominator. That is a countable third quantity, and it is the only place the
"unbounded coverage" pitch can be tested.

## Four counts

Per review, over ~335 scored positions in 50 games:

- **A. False claims.** Adjudicable claims contradicted by the board or the
  oracle. Precision.
- **B. Mechanism recall.** Share of denominator facts the arm stated, reported
  separately for tier 1 and tier 2, with the unclassified-relation count
  alongside.
- **C. Verified extra.** True, adjudicable claims not in the denominator. Where
  coverage beyond the motif set shows up, if it exists.
- **D. Unadjudicable claims.** Counted, not scored — see below.

A alone picks the detector arm. B may too, once the detector arm is given the
PV-consequence facts it needs (below). C is the only count where the LLM arm
can win. All four are needed for the pair of failure modes in PROBLEM.md —
shallow-but-correct against fluent-but-wrong — to be visible at once.

## The access asymmetry, and the fix

Axis A favours the detector arm by construction. Axis C would favour the LLM
arm the same way if only one arm can see the refutation.

The detector arm must therefore ship the PV consequence as one of its detectors:
what the best reply wins, from which square, and by what relation. This is a
verified fact computed from the position and the oracle, so it belongs in that
arm by its own premise, and it is among the cheapest detectors to write. Without
it, the detector arm is structurally unable to state the denominator and axis B
picks the LLM arm before the experiment runs — the same failure as before, in
the opposite direction.

A consequence to accept in advance: if the PV-consequence detector carries most
of tier-1 and tier-2 recall, the fork resolves toward the detector arm for a
reason neither branch of PROBLEM.md anticipated, with the model narrating a fact
list that is mostly one detector's output. That is a result, not a flaw in the
measurement.

## Claims have to be comparable to be counted

All four counts need claims in a form that can be matched against the
denominator and checked against the board. Both arms emit, alongside prose, a
structured claim list: ply, claim type, subject square, object square,
relation. Scoring is then set operations — a claim matching a denominator fact
is recall, a claim contradicting the board is false, a true claim matching
nothing is extra.

This constrains the LLM arm's output, which looks like a handicap. It is not: a
claim that cannot be put in that form cannot be checked for truth either, so it
was already outside the false-claim count. Keeping it outside recall is
consistent treatment, not an extra penalty.

The alternative — free prose scored by a model judge — was rejected. It adds a
judge error term that would itself need validating against the same board facts
the structured form gives directly.

## The blind spot

Claims that are neither board facts nor evaluations — "you abandoned your
queenside plan", "the queen came out too early", "wrong piece to trade" — are
not in the denominator and cannot be contradicted by it. They cost nothing on A
and earn nothing on B or C.

These are also the claims that make a review coaching rather than annotation,
so the measurement is blind to a part of the thing being measured, and blind in
a direction that flatters an arm producing them freely. Count D exists to keep
the volume visible. It is reported, never scored: a number that says how much of
each arm's output this experiment did not assess.

## Turning the counts into a decision

PROBLEM.md already weights the axes: "a review that says less and is correct
beats a richer one that is not." So precision gates and coverage ranks.

- **Gate:** at least 90% of reviews contain zero false claims — on 50 games, at
  most 5 reviews carrying any. The criterion in PROBLEM.md is absolute ("no
  false statements"), which no arm will meet across ~335 positions, so a number
  is needed and is stated here before the run rather than chosen after seeing
  the distribution. 90% is a judgement about what a beginner can survive, not a
  derived figure: a reader in the 400-999 band cannot tell a false claim from a
  true one, so a wrong claim is not self-correcting.
- **Rank:** among arms that pass the gate, order by B, then C.
- **Neither passes:** the fork is not settled by this experiment, and the answer
  is a third design rather than the better of two failures.

## Trade-offs

- **The denominator is tactical.** 83% coverage of mistakes in this band, and
  the missing 17% are positional — the mistakes a coach most needs language for.
  Accepted because the alternative is hand-annotation: ~335 positions at a few
  minutes each is over a day of work, and it makes the metric unrepeatable when
  either arm changes.
- **Tier-2 recall has the detector arm's ceiling.** The measurement cannot see
  further into mechanism than a detector can, so it cannot fully price the LLM
  arm's best case. Count C is the partial compensation, and the
  unclassified-relation count is how much of the question went unreached.
- **Structured claims change what the arms produce.** Neither arm is being
  measured in the form it would ship in. The alternative was a model judge whose
  errors are unbounded and correlated with the thing being judged.
- **Selection uses Lichess evals, scoring uses local Stockfish.** The `%eval`
  tags are variable-depth and only pick candidate positions; every number that
  enters a count comes from local Stockfish at fixed depth, which is bit-exact
  across runs at `Threads 1` with a fresh process per position (SESSION.md,
  2026-08-18). Candidate selection being slightly noisy shifts which positions
  are scored, not what they score.

## Sizing

Measured 2026-08-29 over the 1,269 games in the band that carry `%eval`
(`tmp/blunder_density.py`), evals capped at +/-1000cp before differencing:

| loss threshold | mean per game | median | games with none |
|---|---|---|---|
| >= 100cp | 12.7 | 11 | 3 / 1269 |
| >= 200cp | 6.7 | 5 | 21 / 1269 |
| >= 300cp | 4.1 | 3 | 99 / 1269 |

At >= 200cp, 50 games gives ~335 scored positions, ~278 of them with a
derivable denominator, at 1-3 facts each. The cap matters: 15% of >= 200cp
swings are movements inside already-decided positions, which is the first cut at
the undefined "critical moment" in PROBLEM.md.

## Open decisions

1. **Mistake threshold.** 200cp is assumed above. 300cp gives ~205 positions and
   a cleaner signal; 100cp gives ~635 and includes a great deal of noise at this
   rating.
2. **The 90% gate.** Stated here so it is pre-registered, but it is a judgement
   and worth disagreeing with before the run rather than after.
3. **Fact granularity.** One tier-1 fact per material transfer in the PV, one
   tier-2 fact per relation. Long forcing lines produce several transfers;
   whether a five-ply sequence is one fact or four changes recall denominators
   and is not settled.
4. **PV truncation.** Material is diffed over the whole PV, which is noisy where
   the line ends mid-exchange. A quiescence cutoff would be better and has not
   been specified.
