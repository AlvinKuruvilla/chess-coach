// Turning review data into the words the app shows. Shared by the game
// view and the trainer so both describe a mistake the same way.

import type { Fact, Mistake, Side, Unit } from "./api";

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

/**
 * Sentences saying why a refutation works, from the backend's facts
 * (src-tauri/src/explain.rs), followed by what it costs. Falls back to the
 * cost alone when there are no facts.
 *
 * Every sentence is true of the position it talks about, which constrains
 * the wording: the attacker and defender lists hold at the capture, so they
 * appear only in the sentence about the capture, never in one about an
 * earlier move.
 *
 * `played` is the move being explained (a game move or one tried in the
 * trainer) and `mover` the side that made it; `you` is the player reading,
 * so pieces read as "your" and "their". With no `you`, colours are named.
 */
export function explanation(
  m: { facts: Fact[]; refutation: string[]; cost: Mistake["cost"] },
  played: string,
  mover: Side,
  you: Side | null,
): string[] {
  // In the order things happen on the board: a loose piece by the move that
  // made it loose (your own move counts as before the line), the others by
  // their own move.
  const when = (fact: Fact) =>
    fact.kind === "loose" ? ("at_ply" in fact.cause ? fact.cause.at_ply : -1) : fact.at_ply;
  const facts = [...m.facts].sort((a, b) => when(a) - when(b));
  const sentences = facts.map((fact) => factSentence(fact, m.refutation, played, mover, you));
  if (m.cost.kind === "material" && m.cost.material_swing > 0 && sentences.length > 0) {
    sentences.push(
      `In all it costs ${m.cost.material_swing} point${m.cost.material_swing === 1 ? "" : "s"} of material.`,
    );
  } else if (sentences.length === 0) {
    sentences.push(costSentence(m));
  }
  return sentences;
}

function factSentence(
  fact: Fact,
  line: string[],
  played: string,
  mover: Side,
  you: Side | null,
): string {
  const other: Side = mover === "white" ? "black" : "white";
  const owner = (side: Side) => (you === null ? side : side === you ? "your" : "their");
  const mine = (u: Unit) => `${owner(mover)} ${u.role} on ${u.square}`;
  const theirs = (u: Unit) => `${owner(other)} ${u.role} on ${u.square}`;
  const move = (ply: number) => line[ply] ?? "a later move";

  switch (fact.kind) {
    case "fork": {
      const targets = fact.targets.map((t) =>
        t.check ? `${owner(mover)} king (check)` : `${owner(mover)} ${t.role} on ${t.square}`,
      );
      return `${upTo(line, fact.at_ply)}${move(fact.at_ply)} attacks ${list(targets)} at once.`;
    }
    case "pin":
      return `${upTo(line, fact.at_ply)}${move(fact.at_ply)} pins ${mine(fact.pinned)} to ${owner(mover)} ${fact.behind.role}.`;
    case "loose": {
      const piece = `${owner(mover)} ${fact.piece.role} on ${fact.piece.square}`;
      const attackers = list(fact.attackers.map(theirs));
      const defenders = fact.defenders.length > 0 ? list(fact.defenders.map(mine)) : "nothing";
      const capture = `${upTo(line, fact.at_ply)}${move(fact.at_ply)} wins it: it is attacked by ${attackers} and defended by ${defenders}.`;
      return `${looseCause(fact, line, played, piece, mine, theirs)} ${capture}`;
    }
  }
}

function looseCause(
  fact: Extract<Fact, { kind: "loose" }>,
  line: string[],
  played: string,
  piece: string,
  mine: (u: Unit) => string,
  theirs: (u: Unit) => string,
): string {
  const square = fact.piece.square;
  const cause = fact.cause;
  switch (cause.type) {
    case "moved_there":
      return `${played} put ${piece}.`;
    case "ignored":
      return `${capitalise(piece)} was already loose before ${played}, and ${played} left it there.`;
    case "defender_moved":
      return `${played} took the piece on ${cause.from}, which was defending ${square}, away to ${cause.to}.`;
    case "line_opened":
      // Either an attacker's line opened or a defender's closed; either way
      // the square was safe before the move and is not after it.
      return `${capitalise(piece)} was safe before ${played} and is loose after it.`;
    case "attacker_added":
      return `${upTo(line, cause.at_ply)}${line[cause.at_ply]} adds ${theirs(cause.by)} to the attack on ${piece}.`;
    case "defender_pinned":
      return `${upTo(line, cause.at_ply)}${line[cause.at_ply]} pins ${mine(cause.pinned)}, so it can no longer defend ${square}.`;
    case "defenders_lost":
      return `${upTo(line, cause.at_ply)}${line[cause.at_ply]} leaves ${square} without ${list(cause.lost.map(mine))}.`;
    case "arrived_loose":
      return `${upTo(line, cause.at_ply)}${line[cause.at_ply]} leaves ${piece} loose.`;
  }
}

// "After Nxe3 fxe3, " for a move partway down the line. Longer lead-ins are
// summarised, since a sentence quoting six moves is unreadable.
function upTo(line: string[], ply: number): string {
  if (ply === 0) return "";
  if (ply <= 4) return `After ${line.slice(0, ply).join(" ")}, `;
  return `${Math.ceil(ply / 2)} moves into the line, `;
}

function list(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}
