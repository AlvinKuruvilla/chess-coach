# The evaluation-to-expected-score curve

Mistakes are classified by how much expected score a move gives away, not by
centipawns. The curve that turns an evaluation into an expected score is

```text
expected_score(cp) = 100 / (1 + exp(-k * cp))        percent, for the side to move
k = 0.002671
```

fitted on Lichess games between players rated 400-999. A move is a mistake
when it costs 10 or more points of expected score and a blunder at 15 or more.
The constant is `WIN_CURVE_K` in `app/src-tauri/src/analysis.rs` and
`app/src/eval.ts`; the thresholds are `MISTAKE_POINTS` and `BLUNDER_POINTS`.
Decided 2026-11-01.

## Why not centipawns

A fixed centipawn threshold treats every two-pawn swing alike. On this
account's 11 games (2026-11-01, depth 16, both players' moves counted) the
200cp rule flagged 52 moves and this rule flags 48, agreeing on 41:

- The 11 the old rule flagged and this one does not were all in decided
  positions: +5.2 to +7.3, +12.1 to +7.5, -6.1 to -8.1.
- The 7 this rule adds are mostly swings near equality that fell short of two
  pawns (-0.1 to +1.9, +1.3 to +3.2), and one forced mate given up at +8.

Three of the 11 games are against chess.com's Coach-David bot.

## Why this curve and not Lichess's

Lichess publishes the same form of curve with k = 0.00368208
([scalachess `eval.scala` at b73baa5](https://github.com/lichess-org/scalachess/blob/b73baa55d67b4a3013c8114d5646b6c0d233f42b/core/src/main/scala/eval.scala#L107-L109)),
fitted on about 75k positions from 2300+ rated rapid games of June 2022
([lila PR #11148](https://github.com/lichess-org/lila/pull/11148)). chess.com's
"Expected Points" model depends on rating but is not published.

`scripts/fit_win_curve.py` fits k by rating band on the 63 MB sample of the
2026-07 Lichess database kept in `tmp/` (see SESSION.md for how it was
fetched), with the PR's model and filters: result from White's side against
White's evaluation, abandoned games, time forfeits and games of four moves or
fewer dropped, evaluations capped at +-1000, mates dropped. The 95% intervals
come from resampling whole games, 200 rounds.

| Band | Games | Positions | k | 95% CI |
|---|---|---|---|---|
| 400-999 | 855 | 41,930 | 0.002671 | [0.002403, 0.002987] |
| 400-999, rapid and slower | 456 | 22,539 | 0.002723 | [0.002392, 0.003057] |
| 400-999, time forfeits kept | 1,072 | 53,693 | 0.001926 | [0.001715, 0.002181] |
| 1000-1499 | 3,849 | 213,755 | 0.003159 | [0.003020, 0.003330] |
| 2300+ | 503 | 35,987 | 0.005146 | [0.004513, 0.005849] |

k rises with rating: the same evaluation is worth less to weaker players,
who convert advantages less reliably. Lichess's constant lies outside the
400-999 interval, so the band's own fit is used, as agreed before the fit was
run.

At k = 0.002671 an advantage of one pawn is a 57% expected score, two pawns
63%, five pawns 79%. With Lichess's constant those are 59%, 68% and 86%.

## Thresholds

Lichess's, unchanged: on its -1..1 scale an inaccuracy is a drop of 0.1, a
mistake 0.2 and a blunder 0.3
([lila `Advice.scala` at 9934f0a](https://github.com/lichess-org/lila/blob/9934f0a6efa320e71575308c874f4bff5e00225c/modules/tree/src/main/Advice.scala#L45-L49)),
which is 5, 10 and 15 points of expected score. chess.com publishes 5, 10 and
20. Inaccuracies are not reported.

## Trade-offs and open problems

- **The method did not reproduce Lichess's constant.** On 2300+ games it gives
  0.0051, not 0.0037. Differences from the PR that could explain it: the PR
  used rapid games only (this sample has 15 such games at 2300+, too few to
  fit), 2022 rather than 2026 data, and hand-placed anchor points that pull
  the curve towards older Lichess models. Until the gap is explained, the
  absolute value of k is less certain than its ordering across bands.
- **Time forfeits move k a lot** in this band (0.0027 with them dropped, 0.0019
  kept): low-rated players lose many won positions on the clock. Dropping them
  follows the PR, and suits this account, whose games are all daily
  (correspondence) and never time out in the same way.
- **The account's population is not the fitted one.** Its games are daily and
  its rating is around 250-300, below the band's floor; the sample holds
  almost no correspondence games and few players under 400.
- **Lichess's in-game evaluations come from varying depths**, so the fit
  regresses results on a noisier evaluation than the app's fixed depth 16.
- **Changing k or a threshold changes `CRITERION`**, which invalidates every
  stored review; they are recomputed on next open.
