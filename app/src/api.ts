import { Channel, invoke } from "@tauri-apps/api/core";

// Types mirror the serde output of the commands in src-tauri/src/lib.rs,
// store.rs and analysis.rs.
// XXX: hand-copied; generate them (tauri-specta or ts-rs) once shapes settle.

export type Side = "white" | "black";

export interface GameSummary {
  id: number;
  url: string;
  end_time: number;
  time_class: string;
  rated: boolean;
  white: string;
  black: string;
  white_rating: number;
  black_rating: number;
  white_result: string;
  black_result: string;
  eco_url: string | null;
  has_accuracies: boolean;
  you: Side | null;
}

export interface GameDetail extends GameSummary {
  pgn: string;
  accuracies: { white: number; black: number } | null;
}

export interface GamePage {
  total: number;
  offset: number;
  games: GameSummary[];
}

export interface ImportResult {
  username: string;
  archives_fetched: number;
  archives_skipped: number;
  games_new: number;
}

export type CostKind = "material" | "mate" | "trade_neutral" | "positional";

export interface Mistake {
  ply: number;
  move_number: number;
  side: Side;
  played: string;
  best: string;
  best_line: string[];
  severity: "mistake" | "blunder";
  /** Percentage points of expected score given away; what decides `severity`. */
  chances_lost: number;
  cp_loss: number;
  fen_before: string;
  refutation: string[];
  cost: {
    material_swing: number;
    squares: string[];
    mate_in: number | null;
    kind: CostKind;
  };
}

export interface Review {
  white: string;
  black: string;
  result: string;
  opening: string | null;
  depth: number;
  criterion: string;
  mistake_count: number;
  mistakes: Mistake[];
  /** Centipawns from White's side, one per position (moves + 1). ±10000 is mate. */
  evaluations: number[];
}

export interface Progress {
  done: number;
  total: number;
}

// Commands reject with the backend's error text (an eyre chain as a string).
// Wrapping it in an Error gives callers one shape to catch.
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw new Error(typeof err === "string" ? err : String(err));
  }
}

export const api = {
  account: () => call<string | null>("account"),
  connectAccount: (username: string) => call<ImportResult>("connect_account", { username }),
  syncGames: () => call<ImportResult>("sync_games"),
  games: () => call<GamePage>("list_games", {}),
  game: (id: number) => call<GameDetail>("get_game", { id }),
  storedReview: (id: number) => call<Review | null>("stored_review", { id }),
  unreviewedGames: () => call<number[]>("unreviewed_games"),
  review: (id: number, onProgress: (p: Progress) => void) => {
    const channel = new Channel<Progress>();
    channel.onmessage = onProgress;
    return call<Review>("review_game", { id, onProgress: channel });
  },
};

/** Win, loss, or draw from the importing player's side. */
export function outcome(game: GameSummary): "win" | "loss" | "draw" {
  const mine = game.you === "black" ? game.black_result : game.white_result;
  const theirs = game.you === "black" ? game.white_result : game.black_result;
  if (mine === "win") return "win";
  if (theirs === "win") return "loss";
  return "draw";
}
