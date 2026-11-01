"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { api, outcome, type GamePage, type GameSummary } from "@/lib/api";

export default function GamesPage() {
  const [games, setGames] = useState<GameSummary[] | null>(null);
  const [username, setUsername] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  const show = useCallback((page: GamePage) => {
    setGames(page.games);
    setError(null);
    // Prefill the import box with whoever the stored games were imported
    // for, so re-importing is one click. The backend already knows it.
    const first = page.games.find((g) => g.you);
    if (first?.you) {
      const name = first.you === "white" ? first.white : first.black;
      setUsername((current) => current || name);
    }
  }, []);

  useEffect(() => {
    api.games().then(show, (err: Error) => setError(err.message));
  }, [show]);

  async function runImport(event: React.FormEvent) {
    event.preventDefault();
    if (!username.trim()) return;
    setImporting(true);
    setStatus(null);
    try {
      const result = await api.importGames(username.trim());
      setStatus(
        `${result.games_new} new game${result.games_new === 1 ? "" : "s"} ` +
          `from ${result.archives_fetched} month` +
          `${result.archives_fetched === 1 ? "" : "s"}`,
      );
      show(await api.games());
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setImporting(false);
    }
  }

  return (
    <main className="page">
      <header className="masthead">
        <h1>Games</h1>
        <form className="import" onSubmit={runImport}>
          <input
            aria-label="chess.com username"
            placeholder="chess.com username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
          <button type="submit" disabled={importing || !username.trim()}>
            {importing ? "Importing…" : "Import"}
          </button>
        </form>
      </header>

      {status && <p className="note">{status}</p>}
      {error && <p className="error">{error}</p>}

      {games === null && !error && <p className="note">Loading…</p>}
      {games?.length === 0 && (
        <p className="note">No games yet. Import your chess.com archive above.</p>
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
              const rating = opponentIsBlack
                ? game.black_rating
                : game.white_rating;
              const result = outcome(game);
              return (
                <tr key={game.id}>
                  <td>
                    <Link href={`/games/${game.id}`}>
                      {new Date(game.end_time * 1000).toLocaleDateString()}
                    </Link>
                  </td>
                  <td>
                    <Link href={`/games/${game.id}`}>
                      {opponent} <span className="dim">({rating})</span>
                    </Link>
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
