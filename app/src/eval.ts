// Turning centipawn evaluations into things to draw and read.

/** What the backend uses for a forced mate (`MATE_CP` in analysis.rs). */
export const MATE_CP = 10_000;

/**
 * An evaluation as an expected share of the points, 0 to 1, for the side
 * whose point of view `cp` is from. A logistic curve, so the first two pawns
 * of advantage move the bar a lot and the tenth barely at all -- which
 * matches how much each one changes a game's likely result. The 0.004
 * steepness is chosen by eye, not fitted: one pawn reads as 60%, two as 69%,
 * five as 88%. Lichess uses a logistic of this shape with a fitted constant
 * near 0.0037; it was not checked against the source, so it is not used.
 */
export function share(cp: number): number {
  if (cp >= MATE_CP) return 1;
  if (cp <= -MATE_CP) return 0;
  return 1 / (1 + Math.exp(-0.004 * cp));
}

/** "+1.4", "−0.3", "Mate". `cp` is from the reader's point of view. */
export function formatEval(cp: number): string {
  if (Math.abs(cp) >= MATE_CP) return cp > 0 ? "Mating" : "Getting mated";
  const pawns = cp / 100;
  if (Math.abs(pawns) < 0.05) return "0.0";
  return `${pawns > 0 ? "+" : "−"}${Math.abs(pawns).toFixed(1)}`;
}
