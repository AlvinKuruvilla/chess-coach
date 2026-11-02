import { Chess } from "chess.js";
import { useCallback, useEffect, useMemo, useState } from "react";

import { api, type Drill, type DrillVerdict, type Grade, type PracticeQueue } from "../api";
import { explanation, mark, moveLabel } from "../describe";
import { Board, type Arrow } from "./Board";

/**
 * Where one drill stands.
 *
 * ```text
 *   ask --move--> judging --solved--------------------------> solved
 *                    |
 *                    '--missed, first try--> hint --move--> judging --solved--> solved
 *                                                                |
 *                                                                '--missed--> revealed
 *   "Show answer" from ask or hint ------------------------------------------> revealed
 * ```
 */
type Phase =
  | { kind: "ask"; attempt: 1 | 2; miss?: DrillVerdict }
  | { kind: "judging"; attempt: 1 | 2; uci: string }
  | { kind: "solved"; verdict: DrillVerdict; grade: Grade }
  | { kind: "revealed"; miss?: DrillVerdict };

// A move's from/to squares, played on `fen`, for an arrow.
function arrowFor(fen: string, san: string, brush: Arrow["brush"]): Arrow[] {
  try {
    const move = new Chess(fen).move(san);
    return [{ from: move.from, to: move.to, brush }];
  } catch {
    return [];
  }
}

export function Practice() {
  const [queue, setQueue] = useState<PracticeQueue | null>(null);
  const [index, setIndex] = useState(0);
  const [phase, setPhase] = useState<Phase>({ kind: "ask", attempt: 1 });
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(0);

  const load = useCallback(() => {
    api.practiceQueue().then(
      (q) => {
        setQueue(q);
        setIndex(0);
        setPhase({ kind: "ask", attempt: 1 });
      },
      (err: Error) => setError(err.message),
    );
  }, []);

  useEffect(load, [load]);

  const drill: Drill | undefined = queue?.due[index];

  // Record the outcome once, when a drill reaches an end state.
  async function finish(grade: Grade) {
    if (!drill) return;
    try {
      await api.recordDrill(drill.game_id, drill.mistake.ply, grade);
      setDone((d) => d + 1);
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function tryMove(uci: string) {
    if (!drill || phase.kind !== "ask") return;
    const attempt = phase.attempt;
    setPhase({ kind: "judging", attempt, uci });
    try {
      const verdict = await api.judgeDrill(drill.mistake.fen_before, uci, drill.expected_before);
      if (verdict.solved) {
        const grade: Grade = attempt === 1 ? "first_try" : "retry";
        setPhase({ kind: "solved", verdict, grade });
        await finish(grade);
      } else if (attempt === 1) {
        setPhase({ kind: "ask", attempt: 2, miss: verdict });
      } else {
        setPhase({ kind: "revealed", miss: verdict });
        await finish("missed");
      }
    } catch (err) {
      setError((err as Error).message);
      setPhase({ kind: "ask", attempt });
    }
  }

  async function reveal() {
    if (phase.kind !== "ask") return;
    setPhase({ kind: "revealed", miss: phase.miss });
    await finish("missed");
  }

  function next() {
    setIndex((i) => i + 1);
    setPhase({ kind: "ask", attempt: 1 });
    setError(null);
  }

  const arrows = useMemo<Arrow[]>(() => {
    if (!drill) return [];
    const m = drill.mistake;
    if (phase.kind === "solved" || phase.kind === "revealed") {
      return [
        ...arrowFor(m.fen_before, m.played, "played"),
        ...(m.best ? arrowFor(m.fen_before, m.best, "best") : []),
      ];
    }
    return [];
  }, [drill, phase]);

  if (error && !queue) return <main className="practice"><p className="error">{error}</p></main>;
  if (!queue) return <main className="practice" />;

  if (!drill) {
    return (
      <main className="practice">
        <div className="practice-empty">
          <h1>{done > 0 ? "That's everything due." : "Nothing to practise yet."}</h1>
          <p className="muted">
            {done > 0 && `${done} position${done === 1 ? "" : "s"} done this sitting. `}
            {queue.waiting > 0 &&
              `${queue.waiting} position${queue.waiting === 1 ? " is" : "s are"} waiting to come back. `}
            {queue.unreviewed > 0 &&
              `${queue.unreviewed} game${queue.unreviewed === 1 ? " is" : "s are"} still being reviewed; their mistakes will appear here.`}
            {done === 0 && queue.waiting === 0 && queue.unreviewed === 0 &&
              "Mistakes from your reviewed games appear here once there are some worth drilling."}
          </p>
          <button className="quiet" onClick={load}>Check again</button>
        </div>
      </main>
    );
  }

  const m = drill.mistake;
  const moveNumber = m.move_number;
  const finished = phase.kind === "solved" || phase.kind === "revealed";
  const miss = phase.kind === "ask" || phase.kind === "revealed" ? phase.miss : undefined;

  return (
    <main className="practice">
      <header className="practice-head">
        <h1>Find the move</h1>
        <span className="muted">
          {queue.due.length - index} due
          {queue.waiting > 0 && ` · ${queue.waiting} waiting`}
          {done > 0 && ` · ${done} done`}
        </span>
      </header>

      <div className="practice-body">
        <div className="practice-board">
          <Board
            fen={m.fen_before}
            orientation={drill.you}
            arrows={arrows}
            movable={phase.kind === "ask" ? { color: drill.you, onMove: tryMove } : undefined}
          />
        </div>

        <aside className="practice-panel">
          <p className="context">
            From your game against <strong>{drill.opponent}</strong>,{" "}
            {new Date(drill.end_time * 1000).toLocaleDateString(undefined, {
              day: "numeric",
              month: "short",
            })}
            . You are {drill.you}, move {moveNumber}.
            {drill.seen && " You've seen this one before."}
          </p>

          {phase.kind === "ask" && phase.attempt === 1 && (
            <div className="prompt">
              <h2>Your move.</h2>
              <p>In the game a move here cost you. Find a better one.</p>
            </div>
          )}

          {phase.kind === "ask" && phase.attempt === 2 && miss && (
            <div className="prompt miss">
              <h2>
                {miss.judgement.san} gives away {Math.round(miss.chances_lost)}%.
              </h2>
              {/* Why the tried move fails, from the same detectors as the
                  game's mistakes. It names the punishing reply, not the
                  answer to the position, so the retry is still yours. */}
              <p>{explanation(miss.judgement, miss.judgement.san, drill.you, drill.you).join(" ")}</p>
              <p className="hint">
                Before you move again: what are your checks, captures and threats? And
                your opponent's, after your move?
              </p>
            </div>
          )}

          {phase.kind === "judging" && (
            <div className="prompt">
              <h2 className="muted">Checking…</h2>
            </div>
          )}

          {phase.kind === "solved" && (
            <div className="prompt good">
              <h2>
                {phase.verdict.judgement.san} — {phase.grade === "first_try" ? "found it." : "found it on the retry."}
              </h2>
              <p>
                {phase.verdict.judgement.san === m.best
                  ? "That's the engine's move."
                  : `As good as the engine's ${m.best}, within ${Math.max(1, Math.round(phase.verdict.chances_lost))}%.`}
              </p>
            </div>
          )}

          {phase.kind === "revealed" && (
            <div className="prompt">
              <h2>
                The move was <span className="better">{m.best}</span>.
              </h2>
              {m.best_line.length > 1 && (
                <p className="line">{m.best_line.slice(0, 6).join(" ")}</p>
              )}
            </div>
          )}

          {finished && (
            <div className="what-happened">
              <p>
                In the game you played{" "}
                <strong>
                  {moveLabel(moveNumber, m.side, m.played)}
                  {mark(m)}
                </strong>
                . {explanation(m, m.played, m.side, drill.you).join(" ")}
              </p>
              {m.refutation.length > 0 && (
                <p className="line">{m.refutation.slice(0, 6).join(" ")}</p>
              )}
            </div>
          )}

          {error && <p className="error">{error}</p>}

          <div className="practice-actions">
            {phase.kind === "ask" && (
              <button className="quiet" onClick={() => void reveal()}>
                Show answer
              </button>
            )}
            {finished && (
              <button className="button" onClick={next} autoFocus>
                Next position
              </button>
            )}
          </div>
        </aside>
      </div>
    </main>
  );
}
