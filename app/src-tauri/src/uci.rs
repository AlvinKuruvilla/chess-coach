//! A minimal async driver for a UCI engine: start Stockfish, search one
//! position to a fixed depth, read back the score and principal variation.
//!
//! The `uci` crate (0.2.3) was the obvious dependency and does not fit: it is
//! synchronous, searches only by `movetime`, and returns the best move alone,
//! with no score or PV. Everything needed here is four commands and one kind
//! of `info` line, so the driver is written out.
//!
//! The protocol, as used:
//!
//! ```text
//!   -> uci                      <- ... uciok
//!   -> setoption name Threads value 1
//!   -> isready                  <- readyok
//!   per position:
//!   -> ucinewgame               (clears the hash table)
//!   -> isready                  <- readyok
//!   -> position fen <root> moves <m1> ... <mk>
//!   -> go depth <d>             <- info depth 1 ... score cp 31 ... pv e2e4 ...
//!                               <- info depth <d> ... score cp 25 ... pv ...
//!                               <- bestmove e2e4
//! ```
//!
//! Spec: <https://backscattering.de/chess/uci/> (the de facto copy of the
//! original Stefan Meyer-Kahlen text).

use std::path::Path;
use std::process::Stdio;

use eyre::{Context, bail, eyre};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// An evaluation from the point of view of the side to move, as UCI reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Score {
    Cp(i32),
    /// Moves (not plies) to mate; negative when the side to move is mated.
    Mate(i32),
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub score: Score,
    /// Moves in UCI long algebraic notation (`e2e4`, `e7e8q`).
    pub pv: Vec<String>,
}

pub struct Engine {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Engine {
    pub async fn start(path: &Path) -> eyre::Result<Engine> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // A review dropped mid-search (the window closed, the command was
            // cancelled) must not leave Stockfish running.
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start the engine at {}", path.display()))?;
        let stdin = child.stdin.take().expect("stdin was piped above");
        let stdout = child.stdout.take().expect("stdout was piped above");
        let mut engine = Engine {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
        };

        engine.send("uci").await?;
        engine.wait_for("uciok").await?;
        // One thread and an empty hash per position are the two conditions
        // under which Stockfish at fixed depth was measured to be bit-exact
        // (2026-08-18, SESSION.md). More threads make reviews irreproducible.
        engine.send("setoption name Threads value 1").await?;
        engine.send("isready").await?;
        engine.wait_for("readyok").await?;
        Ok(engine)
    }

    /// Search the position reached from `root_fen` by `moves`, to `depth`.
    ///
    /// The moves are sent rather than the FEN of the final position because
    /// the engine needs the history to see repetitions: from a bare FEN, a
    /// move that repeats the position a third time looks like any other.
    pub async fn analyse(
        &mut self,
        root_fen: &str,
        moves: &[String],
        depth: u32,
    ) -> eyre::Result<Analysis> {
        self.send("ucinewgame").await?;
        self.send("isready").await?;
        self.wait_for("readyok").await?;

        let mut position = format!("position fen {root_fen}");
        if !moves.is_empty() {
            position.push_str(" moves ");
            position.push_str(&moves.join(" "));
        }
        self.send(&position).await?;
        self.send(&format!("go depth {depth}")).await?;

        // Keep the latest score and the latest PV. They usually arrive on the
        // same line; taking each separately matches python-chess, which the
        // previous implementation used, and tolerates engines that split them.
        let mut score = None;
        let mut pv = Vec::new();
        loop {
            let line = self.next_line().await?;
            let mut words = line.split_ascii_whitespace();
            match words.next() {
                Some("bestmove") => break,
                Some("info") => {}
                _ => continue,
            }
            let words: Vec<&str> = words.collect();
            if let Some(found) = parse_score(&words) {
                score = Some(found);
            }
            if let Some(at) = words.iter().position(|w| *w == "pv") {
                pv = words[at + 1..].iter().map(|m| (*m).to_owned()).collect();
            }
        }
        let score = score.ok_or_else(|| eyre!("the engine finished without a score"))?;
        Ok(Analysis { score, pv })
    }

    pub async fn quit(mut self) -> eyre::Result<()> {
        self.send("quit").await?;
        self.child
            .wait()
            .await
            .context("wait for the engine to exit")?;
        Ok(())
    }

    async fn send(&mut self, command: &str) -> eyre::Result<()> {
        self.stdin
            .write_all(format!("{command}\n").as_bytes())
            .await
            .with_context(|| format!("send `{command}` to the engine"))
    }

    async fn next_line(&mut self) -> eyre::Result<String> {
        match self
            .lines
            .next_line()
            .await
            .context("read from the engine")?
        {
            Some(line) => Ok(line),
            None => bail!("the engine exited unexpectedly"),
        }
    }

    async fn wait_for(&mut self, token: &str) -> eyre::Result<()> {
        while self.next_line().await?.trim() != token {}
        Ok(())
    }
}

/// `score cp 31` or `score mate -2` out of an `info` line's words. Bound
/// scores (`score cp 31 lowerbound`) are taken as-is; at the final depth
/// Stockfish reports an exact score.
fn parse_score(words: &[&str]) -> Option<Score> {
    let at = words.iter().position(|w| *w == "score")?;
    let value: i32 = words.get(at + 2)?.parse().ok()?;
    match *words.get(at + 1)? {
        "cp" => Some(Score::Cp(value)),
        "mate" => Some(Score::Mate(value)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_are_read_from_info_lines() {
        let line = "depth 12 seldepth 18 multipv 1 score cp -45 nodes 1 pv e7e5";
        let words: Vec<&str> = line.split_ascii_whitespace().collect();
        assert_eq!(parse_score(&words), Some(Score::Cp(-45)));

        let words: Vec<&str> = "depth 3 score mate -2 pv d8h4".split(' ').collect();
        assert_eq!(parse_score(&words), Some(Score::Mate(-2)));

        let words: Vec<&str> = "depth 3 currmove e2e4 currmovenumber 1"
            .split(' ')
            .collect();
        assert_eq!(parse_score(&words), None);
    }
}
