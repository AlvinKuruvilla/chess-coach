import { useEffect, useState } from "react";

import { api } from "./api";
import { GameView } from "./components/GameView";
import { GamesList } from "./components/GamesList";
import { Practice } from "./components/Practice";
import { Welcome } from "./components/Welcome";

type Tab = "games" | "practice";

// A handful of screens, so plain state rather than a router. The games list
// stays mounted (hidden) while a game or the trainer is open, so its scroll
// position and background reviews carry on. The trainer remounts on each
// visit, so its queue reflects reviews finished meanwhile.
export default function App() {
  // undefined: not known yet; null: no account, show the first-run screen.
  const [account, setAccount] = useState<string | null | undefined>(undefined);
  const [tab, setTab] = useState<Tab>("games");
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
        onConnected={() => api.account().then(setAccount, (err: Error) => setError(err.message))}
      />
    );
  }

  const inGame = openGame !== null;
  return (
    <>
      {!inGame && (
        <nav className="tabs" aria-label="Sections">
          {(["games", "practice"] as const).map((t) => (
            <button key={t} aria-current={tab === t ? "page" : undefined} onClick={() => setTab(t)}>
              {t === "games" ? "Games" : "Practice"}
            </button>
          ))}
        </nav>
      )}
      <div hidden={inGame || tab !== "games"}>
        <GamesList account={account} onOpen={setOpenGame} />
      </div>
      {!inGame && tab === "practice" && <Practice />}
      {inGame && <GameView key={openGame} id={openGame} onBack={() => setOpenGame(null)} />}
    </>
  );
}
