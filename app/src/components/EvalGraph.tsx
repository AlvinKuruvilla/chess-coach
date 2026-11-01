import type { Mistake } from "../api";
import { share } from "../eval";

/**
 * The whole game's evaluation as one line, from your point of view: above
 * the middle you were better, below it worse. Mistakes are marked where
 * they landed. Clicking anywhere jumps the board there.
 *
 * Coordinates: x is the ply (0..plies), y is 0..100 with 0 at the top, so
 * your share s draws at y = 100 * (1 - s).
 */
export function EvalGraph({
  evaluations,
  mistakes,
  current,
  isMine,
  onSeek,
}: {
  /** From your point of view, one per position. */
  evaluations: number[];
  mistakes: Mistake[];
  current: number;
  isMine: (m: Mistake) => boolean;
  onSeek: (ply: number) => void;
}) {
  const plies = Math.max(evaluations.length - 1, 1);
  const y = (cp: number) => 100 * (1 - share(cp));
  const points = evaluations.map((cp, i) => `${i},${y(cp).toFixed(2)}`);
  const area = `M0,50 L${points.join(" L")} L${evaluations.length - 1},50 Z`;

  function seek(event: React.MouseEvent<SVGSVGElement>) {
    const box = event.currentTarget.getBoundingClientRect();
    const fraction = (event.clientX - box.left) / box.width;
    onSeek(Math.round(Math.min(1, Math.max(0, fraction)) * plies));
  }

  return (
    <svg
      className="eval-graph"
      viewBox={`0 0 ${plies} 100`}
      preserveAspectRatio="none"
      onClick={seek}
      role="img"
      aria-label="Evaluation over the game; click to jump to a move"
    >
      <rect x="0" y="0" width={plies} height="50" className="graph-ahead" />
      <path d={area} className="graph-area" />
      <polyline points={points.join(" ")} className="graph-line" />
      <line x1="0" x2={plies} y1="50" y2="50" className="graph-mid" />
      <line x1={current} x2={current} y1="0" y2="100" className="graph-now" />
      {mistakes.map((m) => (
        // The mark sits on the position the mistake produced, where the
        // drop shows. Drawn as a vertical tick rather than a dot because the
        // SVG is stretched non-uniformly and a circle would become an oval.
        <line
          key={m.ply}
          x1={m.ply + 1}
          x2={m.ply + 1}
          y1={y(evaluations[m.ply + 1] ?? 0) - 9}
          y2={y(evaluations[m.ply + 1] ?? 0) + 9}
          className={isMine(m) ? "graph-mistake" : "graph-mistake theirs"}
        />
      ))}
    </svg>
  );
}
