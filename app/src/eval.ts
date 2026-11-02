// Turning centipawn evaluations into things to draw and read.

/** What the backend uses for a forced mate (`MATE_CP` in analysis.rs). */
export const MATE_CP = 10_000;

/**
 * Steepness of the evaluation-to-expected-score curve, fitted on Lichess
 * games between players rated 400-999 (docs/win-curve.md). Must match
 * `WIN_CURVE_K` in src-tauri/src/analysis.rs, which uses it to decide what
 * counts as a mistake; the bar and graph would otherwise disagree with the
 * review about how much a move cost.
 */
export const WIN_CURVE_K = 0.002671;

/**
 * An evaluation as an expected share of the points, 0 to 1, for the side
 * whose point of view `cp` is from. A logistic curve, so the first pawns of
 * advantage move it a lot and later ones barely at all.
 */
export function share(cp: number): number {
  if (cp >= MATE_CP) return 1;
  if (cp <= -MATE_CP) return 0;
  return 1 / (1 + Math.exp(-WIN_CURVE_K * cp));
}

/** "+1.4", "−0.3", "Mate". `cp` is from the reader's point of view. */
export function formatEval(cp: number): string {
  if (Math.abs(cp) >= MATE_CP) return cp > 0 ? "Mating" : "Getting mated";
  const pawns = cp / 100;
  if (Math.abs(pawns) < 0.05) return "0.0";
  return `${pawns > 0 ? "+" : "−"}${Math.abs(pawns).toFixed(1)}`;
}
