import { useState } from "react";

import { api } from "../api";

/**
 * First-run screen: the one time the app asks for the chess.com username.
 * The backend saves it only after an import with it succeeds, so a typo shows
 * chess.com's "no such player" here instead of being remembered.
 */
export function Welcome({ onConnected }: { onConnected: () => void }) {
  const [username, setUsername] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function connect(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.connectAccount(username);
      onConnected();
    } catch (err) {
      setError((err as Error).message);
      setBusy(false);
    }
  }

  return (
    <main className="welcome">
      <h1>
        Every mistake,
        <br />
        <em>explained.</em>
      </h1>
      <p>
        Chess Coach reads your finished games from chess.com, finds the moves
        that cost you, and shows what each one allowed.
      </p>
      <form onSubmit={connect}>
        <div className="field">
          <label htmlFor="username">Your chess.com username</label>
          <input
            id="username"
            autoFocus
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            disabled={busy}
          />
        </div>
        <button className="button" type="submit" disabled={busy || !username.trim()}>
          {busy ? "Fetching games…" : "Continue"}
        </button>
      </form>
      {error && <p className="error">{error}</p>}
    </main>
  );
}
