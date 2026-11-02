// Turning review data into the words the app shows. Shared by the game
// view and the trainer so both describe a mistake the same way.

import type { Mistake, Side } from "./api";

export { formatEval } from "./eval";

// Book notation: `??` for a blunder, `?` for a mistake. The backend decides
// which, by how much expected score the move gave away.
export function mark(m: Mistake): string {
  return m.severity === "blunder" ? "??" : "?";
}

/** "−18%": the expected score a mistake gave away. */
export function chancesLost(m: Mistake): string {
  return `−${Math.round(m.chances_lost)}%`;
}

// One sentence on what the refutation costs, built only from fields the
// backend derived. `trade_neutral` and `positional` say outright that the
// review does not explain the mistake, instead of reading the captures as
// the reason (see `CostKind` in src-tauri/src/analysis.rs).
export function costSentence(m: Pick<Mistake, "cost">): string {
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

export function moveLabel(moveNumber: number, side: Side, san: string): string {
  return `${moveNumber}${side === "white" ? "." : "…"} ${san}`;
}

