import { Chessground } from "chessground";
import type { Api } from "chessground/api";
import type { DrawShape } from "chessground/draw";
import type { Key } from "chessground/types";
import { Chess, type Square } from "chess.js";
import { useEffect, useRef } from "react";

// Base layout and the piece set. The square colours come from styles.css,
// in place of chessground's brown theme.
import "chessground/assets/chessground.base.css";
import "chessground/assets/chessground.cburnett.css";

export type Brush = "played" | "best" | "reply";

export interface Arrow {
  from: string;
  to: string;
  brush: Brush;
}

// Arrow colours, set on chessground's four standard brush names (its
// `DrawBrushes` type requires exactly those) and mapped from what each arrow
// means. Hex, not the oklch tokens in styles.css, because chessground writes
// them into SVG `stroke` attributes, where CSS variables do not resolve.
// They match --mistake, --better and --ink in the light theme.
const BRUSH_FOR: Record<Brush, "red" | "green" | "blue"> = {
  played: "red",
  best: "green",
  reply: "blue",
};

const BRUSHES = {
  red: { key: "r", color: "#8f2b23", opacity: 0.85, lineWidth: 11 },
  green: { key: "g", color: "#2f6d45", opacity: 0.85, lineWidth: 11 },
  blue: { key: "b", color: "#3a3128", opacity: 0.6, lineWidth: 11 },
  yellow: { key: "y", color: "#b07a1f", opacity: 0.8, lineWidth: 11 },
};

/** Lets the player move pieces for `color`; every other move is refused. */
export interface Movable {
  color: "white" | "black";
  /** Called with the move in UCI form, e.g. `e2e4` or `e7e8q`. */
  onMove: (uci: string) => void;
}

// Legal destinations for the side to move, in chessground's shape.
function destinations(fen: string): Map<Key, Key[]> {
  const dests = new Map<Key, Key[]>();
  for (const move of new Chess(fen).moves({ verbose: true })) {
    const list = dests.get(move.from as Key) ?? [];
    list.push(move.to as Key);
    dests.set(move.from as Key, list);
  }
  return dests;
}

/**
 * The board. Chessground owns its DOM, so React mounts it once and pushes
 * every later change through `api.set`, rather than re-rendering. Read-only
 * unless `movable` is given.
 */
export function Board({
  fen,
  orientation,
  lastMove,
  arrows,
  movable,
}: {
  fen: string;
  orientation: "white" | "black";
  lastMove?: [string, string];
  arrows: Arrow[];
  movable?: Movable;
}) {
  const element = useRef<HTMLDivElement>(null);
  const board = useRef<Api | null>(null);
  // Chessground keeps the `after` callback it was given; reading the latest
  // `movable` through a ref keeps it from calling a stale handler.
  const onMove = useRef<Movable["onMove"] | undefined>(undefined);
  onMove.current = movable?.onMove;

  useEffect(() => {
    if (!element.current) return;
    board.current = Chessground(element.current, {
      viewOnly: true,
      coordinates: true,
      animation: { enabled: true, duration: 180 },
      drawable: { brushes: BRUSHES },
    });
    return () => {
      board.current?.destroy();
      board.current = null;
    };
  }, []);

  const color = movable?.color;
  useEffect(() => {
    const turn = fen.split(" ")[1] === "w" ? "white" : "black";
    board.current?.set({
      fen,
      orientation,
      turnColor: turn,
      lastMove: lastMove as Key[] | undefined,
      viewOnly: !color,
      movable: color
        ? {
            free: false,
            color,
            dests: turn === color ? destinations(fen) : new Map(),
            showDests: true,
            events: {
              after: (from, to) => {
                // Promotion always to a queen. Under-promotion is almost
                // never the answer in a beginner's own games, and asking
                // would need a picker. NOTE: a drill whose answer is an
                // under-promotion would be unsolvable.
                const piece = new Chess(fen).get(from as Square);
                const lastRank = to[1] === "8" || to[1] === "1";
                const promotion = piece?.type === "p" && lastRank ? "q" : "";
                onMove.current?.(`${from}${to}${promotion}`);
              },
            },
          }
        : { color: undefined, dests: new Map() },
    });
    board.current?.setAutoShapes(
      arrows.map(
        (a): DrawShape => ({ orig: a.from as Key, dest: a.to as Key, brush: BRUSH_FOR[a.brush] }),
      ),
    );
  }, [fen, orientation, lastMove, arrows, color]);

  return <div ref={element} className="board" />;
}
