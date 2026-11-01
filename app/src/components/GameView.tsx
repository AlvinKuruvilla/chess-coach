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
import { Board, type Arrow } from "./Board";

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
        ? `This allows a forced mate in ${Math.ceil(mate_in / 2)}.`
        : "This allows a forced mate.";
    case "material":
      return `This loses ${material_swing} point${material_swing === 1 ? "" : "s"} of material.`;
    case "trade_neutral":
      return "Material is traded evenly afterwards, so the exchange is not the reason.";
    case "positional":
      return "No material changes hands; the cost is positional and this review cannot name it.";
  }
}

function moveLabel(m: Mistake): string {
  return `${m.move_number}${m.side === "white" ? "." : "…"} ${m.played}`;
}

const STEP_ICONS = {
  start: "M7 5v14M18 6l-7 6 7 6",
  back: "M15 6l-6 6 6 6",
  forward: "M9 6l6 6-6 6",
  end: "M17 5v14M6 6l7 6-7 6",
};

function StepIcon({ path }: { path: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
      <path d={path} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
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

  // Position after `ply` half-moves; 0 is the start position.
  const fen = ply === 0 ? (moves[0]?.before ?? new Chess().fen()) : moves[ply - 1].after;
  const lastMove = useMemo<[string, string] | undefined>(
    () => (ply > 0 ? [moves[ply - 1].from, moves[ply - 1].to] : undefined),
    [moves, ply],
  );

  const byPly = useMemo(
    () => new Map((review?.mistakes ?? []).map((m) => [m.ply, m])),
    [review],
  );
  const shown = useCallback(
    (m: Mistake) => !onlyMine || !game?.you || m.side === game.you,
    [onlyMine, game],
  );

  // Arrows for the mistake at this position, if any. Before the mistake:
  // what was played and what should have been. One ply later, on the
  // position the mistake left: the reply that punishes it.
  const arrows = useMemo<Arrow[]>(() => {
    const before = byPly.get(ply);
    if (before) {
      const played = squares(before.fen_before, before.played);
      const best = before.best ? squares(before.fen_before, before.best) : null;
      return [
        ...(played ? [{ ...played, brush: "played" as const }] : []),
        ...(best ? [{ ...best, brush: "best" as const }] : []),
      ];
    }
    const after = byPly.get(ply - 1);
    if (after?.refutation[0]) {
      const reply = squares(fen, after.refutation[0]);
      return reply ? [{ ...reply, brush: "reply" }] : [];
    }
    return [];
  }, [byPly, ply, fen]);

  const step = useCallback(
    (delta: number) => setPly((p) => Math.min(moves.length, Math.max(0, p + delta))),
    [moves.length],
  );

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "ArrowLeft") step(-1);
      else if (event.key === "ArrowRight") step(1);
      else if (event.key === "Home" || event.key === "ArrowUp") setPly(0);
      else if (event.key === "End" || event.key === "ArrowDown") setPly(moves.length);
      else if (event.key === "Escape") onBack();
      else return;
      event.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, moves.length, onBack]);

  // Keep the current move in view as the board steps through the game.
  useEffect(() => {
    currentMove.current?.scrollIntoView({ block: "nearest" });
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
          <button className="quiet" onClick={onBack}>← All games</button>
        </div>
        {error && <p className="error" style={{ padding: "0 32px" }}>{error}</p>}
      </main>
    );
  }

  const you: Side = game.you ?? "white";
  const them: Side = you === "white" ? "black" : "white";
  const name = (side: Side) => (side === "white" ? game.white : game.black);
  const rating = (side: Side) => (side === "white" ? game.white_rating : game.black_rating);
  const toMove: Side = fen.split(" ")[1] === "w" ? "white" : "black";

  const mine = review?.mistakes.filter((m) => m.side === game.you).length ?? 0;
  const theirs = (review?.mistakes.length ?? 0) - mine;
  const result = outcome(game);

  const player = (side: Side) => {
    return (
      <div className="player" key={side}>
        <span>{name(side)}</span>
        <span className="rating">{rating(side)}</span>
        {toMove === side && ply < moves.length && <span className="to-move">to move</span>}
      </div>
    );
  };

  const moveButton = (index: number) => {
    const mistake = byPly.get(index);
    const isCurrent = ply === index + 1;
    return (
      <button
        key={index}
        ref={isCurrent ? currentMove : undefined}
        className={`move${isCurrent ? " current" : ""}`}
        onClick={() => setPly(index + 1)}
      >
        {moves[index].san}
        {mistake && <span className="mark">{mark(mistake)}</span>}
      </button>
    );
  };

  const note = (m: Mistake) => {
    const selected = ply === m.ply || ply === m.ply + 1;
    return (
      <button
        key={m.ply}
        className={`note${m.side !== game?.you ? " theirs" : ""}${selected ? " selected" : ""}`}
        onClick={() => setPly(m.ply)}
      >
        <span className="lead">
          <span>
            {moveLabel(m)}
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
        {m.refutation.length > 0 && (
          <p className="line">{m.refutation.slice(0, 8).join(" ")}{m.refutation.length > 8 ? " …" : ""}</p>
        )}
      </button>
    );
  };

  return (
    <main className="game-page">
      <div className="topbar">
        <button className="quiet" onClick={onBack}>← All games</button>
        <button className="quiet" onClick={() => void openUrl(game.url)}>
          Open on chess.com ↗
        </button>
      </div>

      <div className="game">
        <section className="stage">
          <div className="stage-inner">
            {player(them)}
            <Board fen={fen} orientation={you} lastMove={lastMove} arrows={arrows} />
            {player(you)}
            <div className="controls">
              <button onClick={() => setPly(0)} disabled={ply === 0} aria-label="Start">
                <StepIcon path={STEP_ICONS.start} />
              </button>
              <button onClick={() => step(-1)} disabled={ply === 0} aria-label="Back">
                <StepIcon path={STEP_ICONS.back} />
              </button>
              <button onClick={() => step(1)} disabled={ply === moves.length} aria-label="Forward">
                <StepIcon path={STEP_ICONS.forward} />
              </button>
              <button onClick={() => setPly(moves.length)} disabled={ply === moves.length} aria-label="End">
                <StepIcon path={STEP_ICONS.end} />
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
                <button className="button" onClick={runReview}>Review game</button>
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

          <div className="moves" aria-label="Moves">
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
