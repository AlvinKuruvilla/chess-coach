// Types mirror the backend's response models in src/chess_coach/backend.py.
// XXX: hand-copied; generate them from /openapi.json once the shapes settle.

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
  threshold_cp: number;
  mistake_count: number;
  mistakes: Mistake[];
}

async function call<T>(method: string, path: string): Promise<T> {
  const response = await fetch(`/api${path}`, { method });
  if (!response.ok) {
    // FastAPI puts the message in `detail`; anything else (a proxy error with
    // the backend down) has no JSON body.
    const body = await response.json().catch(() => null);
    const detail =
      typeof body?.detail === "string"
        ? body.detail
        : `${response.status} ${response.statusText} -- is the backend running?`;
    throw new Error(detail);
  }
  return (await response.json()) as T;
}

export const api = {
  importGames: (username: string) =>
    call<ImportResult>(
      "POST",
      `/games/import?username=${encodeURIComponent(username)}`,
    ),
  games: (limit = 200, offset = 0) =>
    call<GamePage>("GET", `/games?limit=${limit}&offset=${offset}`),
  game: (id: number) => call<GameDetail>("GET", `/games/${id}`),
  review: (id: number) => call<Review>("POST", `/games/${id}/review`),
};

/** Win, loss, or draw from the importing player's side. */
export function outcome(game: GameSummary): "win" | "loss" | "draw" {
  const mine = game.you === "black" ? game.black_result : game.white_result;
  const theirs = game.you === "black" ? game.white_result : game.black_result;
  if (mine === "win") return "win";
  if (theirs === "win") return "loss";
  return "draw";
}
