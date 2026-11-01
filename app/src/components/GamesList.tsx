import { useCallback, useEffect, useMemo, useState } from "react";

import { api, outcome, type GamePage, type GameSummary } from "../api";

const RESULT_LABEL = { win: "Won", loss: "Lost", draw: "Drawn" } as const;

function monthOf(game: GameSummary): string {
  return new Date(game.end_time * 1000).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
}

export function GamesList({
  account,
  onOpen,
}: {
  account: string;
  onOpen: (id: number) => void;
}) {
  const [games, setGames] = useState<GameSummary[] | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);

  const show = useCallback((page: GamePage) => {
    setGames(page.games);
    setError(null);
  }, []);

  // Pull new games from chess.com. Only the newest month is refetched, so on
  // an ordinary launch this is two small requests.
  const sync = useCallback(async () => {
    setSyncing(true);
    setStatus("Checking for new games…");
    try {
      const result = await api.syncGames();
      setStatus(
        result.games_new === 0
          ? "Up to date"
          : `${result.games_new} new game${result.games_new === 1 ? "" : "s"}`,
      );
      show(await api.games());
    } catch (err) {
      // Offline is the usual cause; the stored games are still shown.
      setStatus(null);
      setError(`Could not reach chess.com. ${(err as Error).message}`);
    } finally {
      setSyncing(false);
    }
  }, [show]);

  useEffect(() => {
    api.games().then(show, (err: Error) => setError(err.message));
  }, [show]);

  // Sync once per launch, after the stored list is on screen.
  useEffect(() => {
    void sync();
  }, [sync]);

  // Games arrive newest first, so grouping in order keeps months in order.
  const months = useMemo(() => {
    const groups: { month: string; games: GameSummary[] }[] = [];
    for (const game of games ?? []) {
      const month = monthOf(game);
      let last = groups[groups.length - 1];
      if (last?.month !== month) {
        last = { month, games: [] };
        groups.push(last);
      }
      last.games.push(game);
    }
    return groups;
  }, [games]);

  return (
    <main className="library">
      <header className="library-head">
        <h1>{account}</h1>
        <div className="sync">
          {status && <span>{status}</span>}
          <button className="quiet" onClick={() => void sync()} disabled={syncing}>
            Refresh
          </button>
        </div>
      </header>

      {error && <p className="error">{error}</p>}
      {games?.length === 0 && (
        <p className="empty">
          No finished games yet. Play one on chess.com and it will appear here
          the next time the app starts.
        </p>
      )}

      {months.map(({ month, games }, groupIndex) => (
        <section className="month" key={month}>
          <h2>{month}</h2>
          {games.map((game, index) => {
            const opponentIsBlack = game.you !== "black";
            const result = outcome(game);
            return (
              <button
                key={game.id}
                className="game-row"
                onClick={() => onOpen(game.id)}
                // Stagger the first screenful on load; later rows need none.
                style={{ animationDelay: `${Math.min(groupIndex * 4 + index, 12) * 30}ms` }}
              >
                <span className="date">
                  {new Date(game.end_time * 1000).toLocaleDateString(undefined, {
                    day: "numeric",
                    month: "short",
                  })}
                </span>
                <span
                  className={`side ${game.you ?? "white"}`}
                  title={`You played ${game.you ?? "?"}`}
                />
                <span className="opponent">
                  {opponentIsBlack ? game.black : game.white}
                  <span className="rating">
                    {opponentIsBlack ? game.black_rating : game.white_rating}
                  </span>
                </span>
                <span className="kind">
                  {game.time_class}
                  {game.rated ? "" : " · unrated"}
                </span>
                <span className={`result ${result}`}>{RESULT_LABEL[result]}</span>
              </button>
            );
          })}
        </section>
      ))}
    </main>
  );
}
