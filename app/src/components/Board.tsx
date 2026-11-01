import { Chessground } from "chessground";
import type { Api } from "chessground/api";
import type { Key } from "chessground/types";
import type { DrawShape } from "chessground/draw";
import { useEffect, useRef } from "react";

import "chessground/assets/chessground.base.css";
import "chessground/assets/chessground.brown.css";
import "chessground/assets/chessground.cburnett.css";

export interface Arrow {
  from: string;
  to: string;
  brush: "green" | "red" | "blue" | "yellow";
}

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
      animation: { enabled: true, duration: 150 },
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
        (a): DrawShape => ({
          orig: a.from as Key,
          dest: a.to as Key,
          brush: a.brush,
        }),
      ),
    );
  }, [fen, orientation, lastMove, arrows]);

  return <div ref={element} className="board" />;
}
