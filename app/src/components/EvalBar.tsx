import { formatEval, share } from "../eval";

/**
 * A vertical bar beside the board: the filled part is your share of the
 * expected result, so it grows when the position gets better for you.
 * `cp` is from your point of view; `null` before the game is reviewed. The
 * number itself is in the caption under the board and the tooltip; at this
 * width a label inside the bar would not fit "+10.0".
 */
export function EvalBar({ cp }: { cp: number | null }) {
  const yours = cp === null ? 0.5 : share(cp);
  return (
    <div
      className={`eval-bar${cp === null ? " unknown" : ""}`}
      role="meter"
      aria-label="Evaluation for you"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(yours * 100)}
      title={cp === null ? "Review the game to see evaluations" : formatEval(cp)}
    >
      <div className="eval-fill" style={{ transform: `scaleY(${yours})` }} />
    </div>
  );
}
