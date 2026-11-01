import { openUrl } from "@tauri-apps/plugin-opener";
import { Chess, type Move } from "chess.js";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  api,
  outcome,
  type GameDetail,
  type Mistake,
  type Progress,
  type Review,
  type Side,
} from "../api";
import { formatEval } from "../eval";
import { Board, type Arrow } from "./Board";
import { EvalBar } from "./EvalBar";
import { EvalGraph } from "./EvalGraph";

// How long each move stays on the board during autoplay. Long enough to see
// what moved; a review is for reading, not for watching at speed.
const AUTOPLAY_MS = 1100;

// SAN to a from/to pair, played on `fen`. `null` when the move is not legal
// there, which would mean the backend and chess.js disagree about a position;
// the arrow is dropped rather than drawn somewhere wrong.
function squares(fen: string, san: string): { from: string; to: string } | null {
  try {
    const move = new Chess(fen).move(san);
    return { from: move.from, to: move.to };
  } catch {
    return null;
  }
}

// Book notation: `??` for a blunder, `?` for a mistake. The 300cp line is
// the conventional one; anything flagged at all has already passed the
// review's 200cp threshold.
function mark(m: Mistake): string {
  return m.cp_loss >= 300 ? "??" : "?";
}

// One sentence on what the refutation costs, built only from fields the
// backend derived. `trade_neutral` and `positional` say outright that the
// review does not explain the mistake, instead of reading the captures as
// the reason (see `CostKind` in src-tauri/src/analysis.rs).
function costSentence(m: Mistake): string {
  const { kind, material_swing, mate_in } = m.cost;
  switch (kind) {
    case "mate":
      return mate_in
        ? `It allows a forced mate in ${Math.ceil(mate_in / 2)}.`
        : "It allows a forced mate.";
    case "material":
      return `It loses ${material_swing} point${material_swing === 1 ? "" : "s"} of material.`;
    case "trade_neutral":
      return "Material is traded evenly afterwards, so the exchange is not the reason.";
    case "positional":
      return "No material changes hands; the cost is positional and this review cannot name it.";
  }
}

function moveLabel(moveNumber: number, side: Side, san: string): string {
  return `${moveNumber}${side === "white" ? "." : "…"} ${san}`;
}

/**
 * A line played out on the board away from the game: what punished a
 * mistake, or what should have been played. `step` 0 is the position the
 * line starts from; step k is after its k-th move.
 */
interface Line {
  mistake: Mistake;
  kind: "refutation" | "better";
  step: number;
}

interface LinePosition {
  fen: string;
  move?: { from: string; to: string; san: string };
}

/** The positions along `sans` from `fen`, stopping at the first illegal move. */
function walk(fen: string, sans: string[]): LinePosition[] {
  const chess = new Chess(fen);
  const out: LinePosition[] = [{ fen }];
  for (const san of sans) {
    try {
      const move = chess.move(san);
      out.push({ fen: chess.fen(), move: { from: move.from, to: move.to, san: move.san } });
    } catch {
      break;
    }
  }
  return out;
}

const ICONS = {
  start: "M7 5v14M18 6l-7 6 7 6",
  back: "M15 6l-6 6 6 6",
  play: "M8 5.5v13l11-6.5z",
  pause: "M8 5v14M16 5v14",
  forward: "M9 6l6 6-6 6",
  end: "M17 5v14M6 6l7 6-7 6",
};

function Icon({ path, filled }: { path: string; filled?: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path
        d={path}
        fill={filled ? "currentColor" : "none"}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function GameView({ id, onBack }: { id: number; onBack: () => void }) {
  const [game, setGame] = useState<GameDetail | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ply, setPly] = useState(0);
  const [line, setLine] = useState<Line | null>(null);
  const [playing, setPlaying] = useState(false);
  const [onlyMine, setOnlyMine] = useState(true);
  const currentMove = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    api.game(id).then(setGame, (err: Error) => setError(err.message));
    // Show a stored review straight away; null means not reviewed yet.
    api.storedReview(id).then(setReview, (err: Error) => setError(err.message));
  }, [id]);

  const moves: Move[] = useMemo(() => {
    if (!game) return [];
    const chess = new Chess();
    chess.loadPgn(game.pgn);
    return chess.history({ verbose: true });
  }, [game]);

  const byPly = useMemo(
    () => new Map((review?.mistakes ?? []).map((m) => [m.ply, m])),
    [review],
  );
  const you: Side = game?.you ?? "white";
  const isMine = useCallback((m: Mistake) => m.side === game?.you, [game]);
  const shown = useCallback(
    (m: Mistake) => !onlyMine || !game?.you || isMine(m),
    [onlyMine, game, isMine],
  );

  // The positions of the line being shown, if any.
  const linePositions = useMemo(() => {
    if (!line) return null;
    const m = line.mistake;
    return line.kind === "better"
      ? walk(m.fen_before, m.best_line)
      : walk(moves[m.ply]?.after ?? m.fen_before, m.refutation);
  }, [line, moves]);

  // Everything the board shows, from either the game or the line.
  const view = useMemo(() => {
    if (line && linePositions) {
      const at = linePositions[Math.min(line.step, linePositions.length - 1)];
      return {
        fen: at.fen,
        lastMove: at.move ? ([at.move.from, at.move.to] as [string, string]) : undefined,
        arrows: [] as Arrow[],
      };
    }
    const fen = ply === 0 ? (moves[0]?.before ?? new Chess().fen()) : moves[ply - 1].after;
    const lastMove: [string, string] | undefined =
      ply > 0 ? [moves[ply - 1].from, moves[ply - 1].to] : undefined;

    // Before a mistake: what was played and what should have been. One ply
    // later, on the position it left: the reply that punishes it.
    let arrows: Arrow[] = [];
    const before = byPly.get(ply);
    const after = byPly.get(ply - 1);
    if (before) {
      const played = squares(before.fen_before, before.played);
      const best = before.best ? squares(before.fen_before, before.best) : null;
      arrows = [
        ...(played ? [{ ...played, brush: "played" as const }] : []),
        ...(best ? [{ ...best, brush: "best" as const }] : []),
      ];
    } else if (after?.refutation[0]) {
      const reply = squares(fen, after.refutation[0]);
      arrows = reply ? [{ ...reply, brush: "reply" }] : [];
    }
    return { fen, lastMove, arrows };
  }, [line, linePositions, ply, moves, byPly]);

  const lineEnd = linePositions ? linePositions.length - 1 : 0;
  const atEnd = line ? line.step >= lineEnd : ply >= moves.length;
  const atStart = line ? line.step === 0 : ply === 0;

  const step = useCallback(
    (delta: number) => {
      if (line) {
        setLine({ ...line, step: Math.min(lineEnd, Math.max(0, line.step + delta)) });
      } else {
        setPly((p) => Math.min(moves.length, Math.max(0, p + delta)));
      }
    },
    [line, lineEnd, moves.length],
  );
  const toStart = useCallback(() => (line ? setLine({ ...line, step: 0 }) : setPly(0)), [line]);
  const toEnd = useCallback(
    () => (line ? setLine({ ...line, step: lineEnd }) : setPly(moves.length)),
    [line, lineEnd, moves.length],
  );

  function seek(target: number) {
    setPlaying(false);
    setLine(null);
    setPly(target);
  }

  function showLine(m: Mistake, kind: Line["kind"]) {
    setLine({ mistake: m, kind, step: 0 });
    setPlaying(true);
  }

  function leaveLine() {
    if (!line) return;
    setPlaying(false);
    // Return to the game where the line branched off.
    setPly(line.kind === "better" ? line.mistake.ply : line.mistake.ply + 1);
    setLine(null);
  }

  // Autoplay: advance one move per tick and stop at the end.
  useEffect(() => {
    if (!playing) return;
    if (atEnd) {
      setPlaying(false);
      return;
    }
    const timer = window.setTimeout(() => step(1), AUTOPLAY_MS);
    return () => window.clearTimeout(timer);
  }, [playing, atEnd, step]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const manual = () => setPlaying(false);
      if (event.key === "ArrowLeft") (manual(), step(-1));
      else if (event.key === "ArrowRight") (manual(), step(1));
      else if (event.key === "Home" || event.key === "ArrowUp") (manual(), toStart());
      else if (event.key === "End" || event.key === "ArrowDown") (manual(), toEnd());
      else if (event.key === " ") setPlaying((p) => !p && !atEnd);
      else if (event.key === "Escape") (line ? leaveLine() : onBack());
      else return;
      event.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Keep the current move in view as the board steps through the game.
  useEffect(() => {
    currentMove.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [ply]);

  async function runReview() {
    setReviewing(true);
    setError(null);
    try {
      setReview(await api.review(id, setProgress));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setReviewing(false);
      setProgress(null);
    }
  }

  if (!game) {
    return (
      <main className="game-page">
        <div className="topbar">
          <button className="quiet" onClick={onBack}>
            ← All games
          </button>
        </div>
        {error && <p className="error page-error">{error}</p>}
      </main>
    );
  }

  const them: Side = you === "white" ? "black" : "white";
  const name = (side: Side) => (side === "white" ? game.white : game.black);
  const rating = (side: Side) => (side === "white" ? game.white_rating : game.black_rating);
  const toMove: Side = view.fen.split(" ")[1] === "w" ? "white" : "black";

  // Evaluations from your point of view, for the bar and the graph.
  const yourEvals = review?.evaluations.map((cp) => (you === "white" ? cp : -cp)) ?? null;
  const barCp = line || !yourEvals ? null : (yourEvals[ply] ?? null);

  const mine = review?.mistakes.filter(isMine).length ?? 0;
  const theirs = (review?.mistakes.length ?? 0) - mine;
  const result = outcome(game);

  // The sentence under the board: what the move just played did.
  const caption = (() => {
    if (line && linePositions) {
      const m = line.mistake;
      const title =
        line.kind === "refutation"
          ? `How ${moveLabel(m.move_number, m.side, m.played)}${mark(m)} is punished`
          : `What ${m.best} would have led to`;
      const at = linePositions[line.step];
      return {
        title,
        body: at.move
          ? `Move ${line.step} of ${lineEnd}: ${at.move.san}`
          : "Press play or → to step through the line.",
        tone: "line",
      };
    }
    const upcoming = byPly.get(ply);
    if (upcoming && shown(upcoming)) {
      return {
        title: `Next: ${moveLabel(upcoming.move_number, upcoming.side, upcoming.played)}${mark(upcoming)}`,
        body: upcoming.best
          ? `The red arrow was played; the green arrow, ${upcoming.best}, was better.`
          : "The red arrow is the mistake.",
        tone: "warn",
      };
    }
    if (ply === 0) {
      return {
        title: "Starting position",
        body: review ? "Press play or → to step through the game." : "Review the game to see where it turned.",
        tone: "plain",
      };
    }
    const last = moves[ply - 1];
    const side: Side = last.color === "w" ? "white" : "black";
    const number = Math.floor((ply - 1) / 2) + 1;
    const just = byPly.get(ply - 1);
    if (just && shown(just)) {
      return {
        title: `${moveLabel(number, side, last.san)}${mark(just)}  −${(just.cp_loss / 100).toFixed(1)}`,
        body: `${costSentence(just)}${just.refutation[0] ? ` The arrow shows the reply, ${just.refutation[0]}.` : ""}`,
        tone: "warn",
      };
    }
    return {
      title: moveLabel(number, side, last.san),
      body: barCp === null ? "" : `Evaluation for you: ${formatEval(barCp)}`,
      tone: "plain",
    };
  })();

  const moveButton = (index: number) => {
    const mistake = byPly.get(index);
    const isCurrent = !line && ply === index + 1;
    return (
      <button
        key={index}
        ref={isCurrent ? currentMove : undefined}
        // Even indices are White's moves. Your moves are set in full ink and
        // your opponent's muted, so the column you played reads at a glance.
        className={`move${isCurrent ? " current" : ""}${
          (index % 2 === 0 ? "white" : "black") === game.you ? " mine" : " theirs"
        }`}
        onClick={() => seek(index + 1)}
      >
        {moves[index].san}
        {mistake && <span className="mark">{mark(mistake)}</span>}
      </button>
    );
  };

  const note = (m: Mistake) => {
    const selected = line?.mistake.ply === m.ply || (!line && (ply === m.ply || ply === m.ply + 1));
    return (
      <article
        key={m.ply}
        className={`note${isMine(m) ? "" : " theirs"}${selected ? " selected" : ""}`}
      >
        <button className="note-jump" onClick={() => seek(m.ply)}>
          <span className="lead">
            <span>
              {moveLabel(m.move_number, m.side, m.played)}
              {mark(m)}
            </span>
            <span className="loss">−{(m.cp_loss / 100).toFixed(1)}</span>
          </span>
          <p>
            {m.best && (
              <>
                Better was <span className="better">{m.best}</span>.{" "}
              </>
            )}
            {costSentence(m)}
          </p>
        </button>
        <div className="note-actions">
          {m.refutation.length > 0 && (
            <button
              className={line?.mistake.ply === m.ply && line.kind === "refutation" ? "active" : ""}
              onClick={() => showLine(m, "refutation")}
            >
              ▸ Play the punishment
            </button>
          )}
          {m.best_line.length > 0 && (
            <button
              className={line?.mistake.ply === m.ply && line.kind === "better" ? "active" : ""}
              onClick={() => showLine(m, "better")}
            >
              ▸ Play the better line
            </button>
          )}
        </div>
      </article>
    );
  };

  return (
    <main className="game-page">
      <div className="topbar">
        <button className="quiet" onClick={onBack}>
          ← All games
        </button>
        <button className="quiet" onClick={() => void openUrl(game.url)}>
          Open on chess.com ↗
        </button>
      </div>

      <div className="game">
        <section className="stage">
          <div className="stage-inner">
            <div className="player">
              <span>{name(them)}</span>
              <span className="rating">{rating(them)}</span>
              {toMove === them && !atEnd && <span className="to-move">to move</span>}
            </div>
            <div className={`board-frame${line ? " in-line" : ""}`}>
              <EvalBar cp={barCp} />
              <Board fen={view.fen} orientation={you} lastMove={view.lastMove} arrows={view.arrows} />
            </div>
            <div className="player">
              <span>{name(you)}</span>
              <span className="rating">{rating(you)}</span>
              {toMove === you && !atEnd && <span className="to-move">to move</span>}
            </div>

            <div className={`caption ${caption.tone}`} aria-live="polite">
              <div className="caption-text">
                <strong>{caption.title}</strong>
                {caption.body && <span>{caption.body}</span>}
              </div>
              {line && (
                <button className="quiet" onClick={leaveLine}>
                  Back to game
                </button>
              )}
            </div>

            <div className="controls">
              <button onClick={() => (setPlaying(false), toStart())} disabled={atStart} aria-label="Start">
                <Icon path={ICONS.start} />
              </button>
              <button onClick={() => (setPlaying(false), step(-1))} disabled={atStart} aria-label="Back">
                <Icon path={ICONS.back} />
              </button>
              <button
                className="play"
                onClick={() => setPlaying((p) => !p)}
                disabled={atEnd && !playing}
                aria-label={playing ? "Pause" : "Play"}
              >
                <Icon path={playing ? ICONS.pause : ICONS.play} filled={!playing} />
              </button>
              <button onClick={() => (setPlaying(false), step(1))} disabled={atEnd} aria-label="Forward">
                <Icon path={ICONS.forward} />
              </button>
              <button onClick={() => (setPlaying(false), toEnd())} disabled={atEnd} aria-label="End">
                <Icon path={ICONS.end} />
              </button>
            </div>
          </div>
        </section>

        <aside className="notes">
          <header className="notes-head">
            <h1>
              {game.white} <span className="vs">v</span> {game.black}
            </h1>
            <p>
              {new Date(game.end_time * 1000).toLocaleDateString(undefined, {
                day: "numeric",
                month: "long",
                year: "numeric",
              })}
              {" · "}
              {game.time_class}
              {" · "}
              <span className={`result ${result}`}>
                {result === "win" ? "You won" : result === "loss" ? "You lost" : "Drawn"}
              </span>
              {game.accuracies &&
                ` · accuracy ${game.accuracies[you].toFixed(0)}% vs ${game.accuracies[them].toFixed(0)}%`}
            </p>
          </header>

          <div className="review-bar">
            {!review && !reviewing && (
              <>
                <span className="muted">Not reviewed yet.</span>
                <button className="button" onClick={runReview}>
                  Review game
                </button>
              </>
            )}
            {reviewing && (
              <div className="progress" role="status">
                <span>
                  {progress
                    ? `Analysing position ${progress.done} of ${progress.total}`
                    : "Starting Stockfish…"}
                </span>
                <div className="track">
                  <div
                    className="fill"
                    style={{ transform: `scaleX(${progress ? progress.done / progress.total : 0})` }}
                  />
                </div>
              </div>
            )}
            {review && (
              <>
                <span className="summary">
                  {mine === 0 ? (
                    "No mistakes by you"
                  ) : (
                    <>
                      <span className="count">{mine}</span> mistake{mine === 1 ? "" : "s"} by you
                    </>
                  )}
                </span>
                {game.you && theirs > 0 && (
                  <div className="toggle" role="group" aria-label="Which mistakes to annotate">
                    <button aria-pressed={onlyMine} onClick={() => setOnlyMine(true)}>
                      Yours
                    </button>
                    <button aria-pressed={!onlyMine} onClick={() => setOnlyMine(false)}>
                      Both
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
          {error && <p className="error">{error}</p>}

          {yourEvals && review && (
            <div className="graph-wrap">
              <EvalGraph
                evaluations={yourEvals}
                mistakes={review.mistakes.filter(shown)}
                current={line ? (line.kind === "better" ? line.mistake.ply : line.mistake.ply + 1) : ply}
                isMine={isMine}
                onSeek={seek}
              />
              <div className="graph-legend">
                <span>better for you</span>
                <span>worse</span>
              </div>
            </div>
          )}

          <div className="moves" aria-label="Moves">
            <div className="move-head">
              <span />
              {(["white", "black"] as const).map((side) => (
                <span key={side} className={side === game.you ? "mine" : ""}>
                  <span className={`side ${side}`} aria-hidden="true" />
                  <span className="who">{name(side)}</span>
                  {side === game.you && <span className="you-tag">you</span>}
                </span>
              ))}
            </div>
            {Array.from({ length: Math.ceil(moves.length / 2) }, (_, row) => {
              const white = 2 * row;
              const black = white + 1;
              const notes = [byPly.get(white), byPly.get(black)].filter(
                (m): m is Mistake => m !== undefined && shown(m),
              );
              return (
                <Fragment key={row}>
                  <div className="move-row">
                    <span className="number">{row + 1}.</span>
                    {moveButton(white)}
                    {black < moves.length ? moveButton(black) : <span />}
                  </div>
                  {notes.map(note)}
                </Fragment>
              );
            })}
          </div>
        </aside>
      </div>
    </main>
  );
}
