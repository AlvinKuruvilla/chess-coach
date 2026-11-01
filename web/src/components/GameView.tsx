"use client";

import { Chess, type Move } from "chess.js";
import Link from "next/link";
import { use, useCallback, useEffect, useMemo, useState } from "react";

import { Board, type Arrow } from "@/components/Board";
import {
  api,
  outcome,
  type GameDetail,
  type Mistake,
  type Review,
} from "@/lib/api";

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

// One line saying what the refutation costs, built only from fields the
// backend derived. `trade_neutral` and `positional` say outright that this
// review does not explain the mistake, instead of reading the captures as
// the reason (see the `kind` docs in src/chess_coach/analysis.py).
function costLine(m: Mistake): string {
  const { kind, material_swing, squares: where, mate_in } = m.cost;
  switch (kind) {
    case "mate":
      return `Allows a forced mate${mate_in ? ` (${mate_in} plies)` : ""}.`;
    case "material":
      return `The refutation wins ${material_swing} point${
        material_swing === 1 ? "" : "s"
      } of material${where.length ? ` (${where.join(", ")})` : ""}.`;
    case "trade_neutral":
      return "Material is traded evenly; the exchange is not why this lost.";
    case "positional":
      return "No material changes hands. This review cannot say why it lost.";
  }
}

export function GameView({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const id = Number(use(params).id);

  const [game, setGame] = useState<GameDetail | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ply, setPly] = useState(0);
  const [onlyMine, setOnlyMine] = useState(true);

  useEffect(() => {
    api.game(id).then(setGame, (err: Error) => setError(err.message));
  }, [id]);

  const moves: Move[] = useMemo(() => {
    if (!game) return [];
    const chess = new Chess();
    chess.loadPgn(game.pgn);
    return chess.history({ verbose: true });
  }, [game]);

  // Position after `ply` half-moves; 0 is the start position.
  const fen =
    ply === 0 ? (moves[0]?.before ?? new Chess().fen()) : moves[ply - 1].after;
  const lastMove = useMemo<[string, string] | undefined>(
    () => (ply > 0 ? [moves[ply - 1].from, moves[ply - 1].to] : undefined),
    [moves, ply],
  );

  const mistakes = useMemo(
    () =>
      (review?.mistakes ?? []).filter(
        (m) => !onlyMine || !game?.you || m.side === game.you,
      ),
    [review, onlyMine, game],
  );

  // Arrows for the mistake at this position, if any. Before the mistake: what
  // was played (red) and what should have been (green). One ply later, on the
  // position the mistake left: the refutation's first move (blue).
  const arrows = useMemo<Arrow[]>(() => {
    const before = review?.mistakes.find((m) => m.ply === ply);
    if (before) {
      const played = squares(before.fen_before, before.played);
      const best = before.best ? squares(before.fen_before, before.best) : null;
      return [
        ...(played ? [{ ...played, brush: "red" as const }] : []),
        ...(best ? [{ ...best, brush: "green" as const }] : []),
      ];
    }
    const after = review?.mistakes.find((m) => m.ply + 1 === ply);
    if (after && after.refutation[0]) {
      const reply = squares(fen, after.refutation[0]);
      return reply ? [{ ...reply, brush: "blue" }] : [];
    }
    return [];
  }, [review, ply, fen]);

  const step = useCallback(
    (delta: number) =>
      setPly((p) => Math.min(moves.length, Math.max(0, p + delta))),
    [moves.length],
  );

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "ArrowLeft") step(-1);
      else if (event.key === "ArrowRight") step(1);
      else if (event.key === "Home") setPly(0);
      else if (event.key === "End") setPly(moves.length);
      else return;
      event.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, moves.length]);

  async function runReview() {
    setReviewing(true);
    setError(null);
    try {
      setReview(await api.review(id));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setReviewing(false);
    }
  }

  if (error && !game) {
    return (
      <main className="page">
        <Link href="/">← Games</Link>
        <p className="error">{error}</p>
      </main>
    );
  }
  if (!game) return <main className="page note">Loading…</main>;

  const mistakePlies = new Set(review?.mistakes.map((m) => m.ply));

  return (
    <main className="page">
      <header className="masthead">
        <div>
          <Link href="/">← Games</Link>
          <h1>
            {game.white} <span className="dim">({game.white_rating})</span> vs{" "}
            {game.black} <span className="dim">({game.black_rating})</span>
          </h1>
          <p className="dim">
            {new Date(game.end_time * 1000).toLocaleDateString()} ·{" "}
            {game.time_class} · you {outcome(game)}
            {game.accuracies &&
              ` · chess.com accuracy ${game.accuracies.white} / ${game.accuracies.black}`}{" "}
            · <a href={game.url}>on chess.com</a>
          </p>
        </div>
      </header>

      <div className="game">
        <section className="board-column">
          <Board
            fen={fen}
            orientation={game.you ?? "white"}
            lastMove={lastMove}
            arrows={arrows}
          />
          <div className="controls">
            <button onClick={() => setPly(0)} aria-label="Start">⏮</button>
            <button onClick={() => step(-1)} aria-label="Back">◀</button>
            <span className="dim">
              {ply} / {moves.length}
            </span>
            <button onClick={() => step(1)} aria-label="Forward">▶</button>
            <button onClick={() => setPly(moves.length)} aria-label="End">⏭</button>
          </div>
        </section>

        <section className="moves" aria-label="Moves">
          <ol>
            {Array.from({ length: Math.ceil(moves.length / 2) }, (_, i) => (
              <li key={i}>
                {[2 * i, 2 * i + 1].map((index) =>
                  index < moves.length ? (
                    <button
                      key={index}
                      className={[
                        "move",
                        ply === index + 1 ? "current" : "",
                        mistakePlies.has(index) ? "mistake" : "",
                      ].join(" ")}
                      onClick={() => setPly(index + 1)}
                    >
                      {moves[index].san}
                    </button>
                  ) : null,
                )}
              </li>
            ))}
          </ol>
        </section>

        <section className="review" aria-label="Review">
          {!review && (
            <button onClick={runReview} disabled={reviewing}>
              {reviewing ? "Reviewing… (up to a minute)" : "Review this game"}
            </button>
          )}
          {error && <p className="error">{error}</p>}
          {review && (
            <>
              <div className="review-head">
                <h2>
                  {mistakes.length} mistake{mistakes.length === 1 ? "" : "s"}
                </h2>
                {game.you && (
                  <label>
                    <input
                      type="checkbox"
                      checked={onlyMine}
                      onChange={(e) => setOnlyMine(e.target.checked)}
                    />{" "}
                    only mine
                  </label>
                )}
              </div>
              <p className="dim">
                Moves losing at least {review.threshold_cp / 100} pawns, depth{" "}
                {review.depth}.
              </p>
              <ul className="mistakes">
                {mistakes.map((m) => (
                  <li key={m.ply}>
                    <button
                      className={ply === m.ply || ply === m.ply + 1 ? "selected" : ""}
                      onClick={() => setPly(m.ply)}
                    >
                      <span className="mistake-title">
                        {m.move_number}
                        {m.side === "white" ? "." : "..."} {m.played}
                        <span className="loss">−{(m.cp_loss / 100).toFixed(1)}</span>
                      </span>
                      {m.best && (
                        <span>
                          Better: <strong>{m.best}</strong>
                        </span>
                      )}
                      <span>{costLine(m)}</span>
                      {m.refutation.length > 0 && (
                        <span className="dim">
                          Refutation: {m.refutation.join(" ")}
                        </span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      </div>
    </main>
  );
}
