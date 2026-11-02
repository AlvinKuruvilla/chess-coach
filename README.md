# Chess Coach

A desktop app that reviews my chess.com games and explains why each mistake
was a mistake.

`app/` is a Tauri app. The Rust side (`app/src-tauri/src`) imports games
from chess.com, stores them in SQLite, and reviews them with Stockfish; the
webview (`app/src`) is React and shows the review on a board.

## Running

Needs `stockfish` (`brew install stockfish`), Rust, and Node.

```bash
cd app
npm install
npm run tauri dev      # development, with reload
npm run tauri build    # a .app bundle under src-tauri/target/release/bundle
```

On first launch the app asks for a chess.com username, once. After that it
pulls new games each time it starts. Data lives in
`~/Library/Application Support/dev.alvinkuruvilla.chesscoach/`.

## Checks

```bash
cd app/src-tauri
cargo test             # engine tests need stockfish; without it they return early
cargo clippy --all-targets
cd .. && npx tsc --noEmit
```

## Licence

GPL-3.0-or-later; see `LICENSE`. The board is
[chessground](https://github.com/lichess-org/chessground), which is
GPL-3.0-or-later, so any build is GPL regardless, and the source carries the
same licence.
