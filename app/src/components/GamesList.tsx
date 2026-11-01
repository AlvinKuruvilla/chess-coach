import { useCallback, useEffect, useState } from "react";

import { api, outcome, type GamePage, type GameSummary } from "../api";

export function GamesList({ onOpen }: { onOpen: (id: number) => void }) {
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
    setStatus("Checking chess.com for new games…");
    try {
      const result = await api.syncGames();
      setStatus(
        result.games_new === 0
          ? "Up to date."
          : `${result.games_new} new game${result.games_new === 1 ? "" : "s"}.`,
      );
      show(await api.games());
    } catch (err) {
      // Offline is the usual cause; the stored games are still shown.
      setStatus(null);
      setError(`Could not reach chess.com: ${(err as Error).message}`);
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

  return (
    <main className="page">
      <header className="masthead">
        <h1>Games</h1>
        <button onClick={() => void sync()} disabled={syncing}>
          {syncing ? "Syncing…" : "Refresh"}
        </button>
      </header>

      {status && <p className="note">{status}</p>}
      {error && <p className="error">{error}</p>}

      {games === null && !error && <p className="note">Loading…</p>}
      {games?.length === 0 && (
        <p className="note">No finished games on chess.com yet.</p>
      )}

      {games && games.length > 0 && (
        <table className="games">
          <thead>
            <tr>
              <th>Date</th>
              <th>Opponent</th>
              <th>Colour</th>
              <th>Result</th>
              <th>Type</th>
            </tr>
          </thead>
          <tbody>
            {games.map((game) => {
              const opponentIsBlack = game.you !== "black";
              const opponent = opponentIsBlack ? game.black : game.white;
              const rating = opponentIsBlack ? game.black_rating : game.white_rating;
              const result = outcome(game);
              return (
                <tr key={game.id} className="clickable" onClick={() => onOpen(game.id)}>
                  <td>{new Date(game.end_time * 1000).toLocaleDateString()}</td>
                  <td>
                    {opponent} <span className="dim">({rating})</span>
                  </td>
                  <td>{game.you ?? "?"}</td>
                  <td className={`result ${result}`}>{result}</td>
                  <td className="dim">
                    {game.time_class}
                    {game.rated ? "" : ", unrated"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </main>
  );
}
