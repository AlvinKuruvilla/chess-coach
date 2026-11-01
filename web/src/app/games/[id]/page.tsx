import { Suspense } from "react";

import { GameView } from "@/components/GameView";

// `params` is request-time data. With Cache Components on, reading it outside
// a Suspense boundary blocks prerendering of the whole route, so the client
// view that reads it sits inside one and the shell prerenders around it.
export default function GamePage({ params }: PageProps<"/games/[id]">) {
  return (
    <Suspense fallback={<main className="page note">Loading…</main>}>
      <GameView params={params} />
    </Suspense>
  );
}
