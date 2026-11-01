import { Chessground } from "chessground";
import type { Api } from "chessground/api";
import type { DrawShape } from "chessground/draw";
import type { Key } from "chessground/types";
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

/**
 * A view-only board. Chessground owns its DOM, so React mounts it once and
 * pushes every later change through `api.set`, rather than re-rendering.
 */
export function Board({
  fen,
  orientation,
  lastMove,
  arrows,
}: {
  fen: string;
  orientation: "white" | "black";
  lastMove?: [string, string];
  arrows: Arrow[];
}) {
  const element = useRef<HTMLDivElement>(null);
  const board = useRef<Api | null>(null);

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

  useEffect(() => {
    board.current?.set({
      fen,
      orientation,
      lastMove: lastMove as Key[] | undefined,
    });
    board.current?.setAutoShapes(
      arrows.map(
        (a): DrawShape => ({ orig: a.from as Key, dest: a.to as Key, brush: BRUSH_FOR[a.brush] }),
      ),
    );
  }, [fen, orientation, lastMove, arrows]);

  return <div ref={element} className="board" />;
}
