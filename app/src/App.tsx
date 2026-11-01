import { useEffect, useState } from "react";

import { api } from "./api";
import { GameView } from "./components/GameView";
import { GamesList } from "./components/GamesList";
import { Welcome } from "./components/Welcome";

// Three screens, so plain state rather than a router. The list stays mounted
// (hidden) while a game is open, so going back keeps its scroll and does not
// sync with chess.com again.
export default function App() {
  // undefined: not known yet; null: no account, show the first-run screen.
  const [account, setAccount] = useState<string | null | undefined>(undefined);
  const [openGame, setOpenGame] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.account().then(setAccount, (err: Error) => setError(err.message));
  }, []);

  if (error) return <main className="page error">{error}</main>;
  if (account === undefined) return null;
  if (account === null) {
    return (
      <Welcome
        onConnected={() =>
          api.account().then(setAccount, (err: Error) => setError(err.message))
        }
      />
    );
  }
  return (
    <>
      <div hidden={openGame !== null}>
        <GamesList account={account} onOpen={setOpenGame} />
      </div>
      {openGame !== null && (
        <GameView key={openGame} id={openGame} onBack={() => setOpenGame(null)} />
      )}
    </>
  );
}
