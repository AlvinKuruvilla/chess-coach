//! Finding the mistakes in a game and saying what each one cost.
//!
//! A game is reviewed by evaluating every position in it once. That single
//! pass yields all three things a mistake report needs: the evaluation swing
//! that identifies a mistake, the move that should have been played instead,
//! and -- from the evaluation of the position the mistake led to -- the
//! refutation and what it takes off the board.
//!
//! The consequence is derived, never asserted. [`Cost`] holds a material diff
//! computed by playing Stockfish's principal variation forward, so a reader
//! cannot be told about a tactic that is not on the board. Measured over 150
//! sampled mistakes in the 400-999 band, 83% carry a consequence derivable
//! this way; the rest are positional and come back empty. That figure was
//! measured on the Python implementation this module was ported from
//! (`src/chess_coach/analysis.py` in commit `9e74157`), see
//! `docs/experiment-metric.md`.

use std::io::Cursor;
use std::ops::ControlFlow;
use std::path::Path;

use eyre::{Context, bail, eyre};
use pgn_reader::{RawTag, Reader, SanPlus, Visitor};
use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Color, EnPassantMode, KnownOutcome, Move, Outcome, Position};

use crate::uci::{Engine, Score};

/// Evaluations are clamped to this before differencing. Without it a swing
/// from +9 to +3 registers as a six-pawn mistake, when the position was
/// winning before and after and nothing a coach would explain has happened.
/// 15% of >=200cp swings in the corpus are of this kind
/// (docs/experiment-metric.md).
const EVAL_CAP_CP: i32 = 1000;

/// What a mate is worth when a mate score has to become a number. Far outside
/// the cap, so any position with a forced mate clamps to the extreme.
const MATE_CP: i32 = 10_000;

/// Pawn-unit piece values for the material diff. The king is absent because
/// it never leaves the board, and a mate is reported separately.
fn material(pos: &Chess, side: Color) -> i32 {
    let m = pos.board().material_side(side);
    i32::from(m.pawn)
        + 3 * i32::from(m.knight)
        + 3 * i32::from(m.bishop)
        + 5 * i32::from(m.rook)
        + 9 * i32::from(m.queen)
}

/// Which of four shapes a consequence has. Shares measured over 150 sampled
/// mistakes; see docs/experiment-metric.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostKind {
    /// The refutation nets material. 81%.
    Material,
    /// The refutation forces mate. 2%.
    Mate,
    /// Material changes hands and washes out. The mistake is real but its
    /// cost is not the exchange, so the captures must not be narrated as the
    /// reason. 6%.
    TradeNeutral,
    /// The refutation moves no material at all. Nothing here explains the
    /// mistake; say that rather than inventing a mechanism. 11%.
    Positional,
}

/// What the refutation takes, derived from the principal variation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    /// Pawns the mistaking side ends up down over the course of the PV.
    pub material_swing: i32,
    /// Squares where material changes hands, in the order the PV takes them.
    pub squares: Vec<String>,
    /// Plies to mate against the mistaking side, when the PV forces one.
    pub mate_in: Option<u32>,
    pub kind: CostKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mistake {
    /// Half-moves played before this one; 0 is White's first move.
    pub ply: usize,
    pub move_number: u32,
    pub side: Side,
    pub played: String,
    /// Empty when the engine had no move to suggest.
    pub best: String,
    /// The engine's line from the position before the mistake, starting with
    /// `best`: what should have happened instead.
    pub best_line: Vec<String>,
    pub cp_loss: i32,
    pub fen_before: String,
    pub refutation: Vec<String>,
    pub cost: Cost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    White,
    Black,
}

impl From<Color> for Side {
    fn from(color: Color) -> Side {
        match color {
            Color::White => Side::White,
            Color::Black => Side::Black,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub white: String,
    pub black: String,
    pub result: String,
    pub opening: Option<String>,
    pub depth: u32,
    pub threshold_cp: i32,
    pub mistake_count: usize,
    pub mistakes: Vec<Mistake>,
    /// The evaluation of every position in the game, in centipawns from
    /// White's point of view: index i is the position after i plies, so
    /// there is one more entry than there are moves. `MATE_CP` (10000) or
    /// its negation means a forced mate; values are not clamped, so the
    /// reader decides how to draw them.
    pub evaluations: Vec<i32>,
}

/// Play the refutation forward and diff the board.
///
/// `after` is the position the mistake produced, so the side to move is the
/// opponent -- the one about to collect. Everything here is a fact about
/// squares and pieces; no evaluation enters.
pub fn consequence_of(after: &Chess, pv: &[Move]) -> Cost {
    let mistaker = !after.turn();
    let balance = |pos: &Chess| material(pos, mistaker) - material(pos, !mistaker);
    let before = balance(after);

    let mut walk = after.clone();
    let mut squares = Vec::new();
    for (plies, m) in (1..).zip(pv) {
        if m.is_capture() {
            squares.push(m.to().to_string());
        }
        walk.play_unchecked(*m);
        if walk.is_checkmate() && walk.turn() == mistaker {
            return Cost {
                material_swing: before - balance(&walk),
                squares,
                mate_in: Some(plies),
                kind: CostKind::Mate,
            };
        }
    }

    let swing = before - balance(&walk);
    let kind = if swing >= 1 {
        CostKind::Material
    } else if !squares.is_empty() {
        CostKind::TradeNeutral
    } else {
        CostKind::Positional
    };
    Cost {
        material_swing: swing,
        squares,
        mate_in: None,
        kind,
    }
}

/// One evaluation as centipawns from White's point of view. UCI reports
/// scores from the side to move's point of view, so Black's are negated.
fn white_cp(score: Score, turn: Color) -> i32 {
    let for_mover = match score {
        Score::Cp(cp) => cp,
        Score::Mate(n) if n > 0 => MATE_CP,
        // `mate 0` is never seen: terminal positions are not searched.
        Score::Mate(_) => -MATE_CP,
    };
    match turn {
        Color::White => for_mover,
        Color::Black => -for_mover,
    }
}

/// The evaluation of a finished position, or `None` if the game continues.
///
/// Terminal positions are never handed to the engine. A checkmate scores as
/// `mate 0`, and zero carries no sign, so the usual mate handling cannot tell
/// who won and reads a delivered mate as a catastrophic blunder by the player
/// who delivered it. Deciding it from the board avoids the question.
///
/// NOTE: shakmaty's `outcome` knows checkmate, stalemate and insufficient
/// material, but not fivefold repetition or the 75-move rule, which need the
/// game history. python-chess counted those; here such a position is searched
/// instead, and the engine (which is given the history) scores it as a draw.
fn terminal_cp(pos: &Chess) -> Option<i32> {
    match pos.outcome() {
        Outcome::Unknown => None,
        Outcome::Known(KnownOutcome::Draw) => Some(0),
        Outcome::Known(KnownOutcome::Decisive { winner }) => Some(match winner {
            Color::White => MATE_CP,
            Color::Black => -MATE_CP,
        }),
    }
}

fn fen(pos: &Chess) -> String {
    Fen::from_position(pos, EnPassantMode::Legal).to_string()
}

/// SAN with its check or mate suffix (`Qxf7#`), as a player writes it.
fn san(pos: &Chess, m: &Move) -> String {
    SanPlus::from_move(pos.clone(), *m).to_string()
}

/// A parsed game: the tags this module reports, the starting position, and
/// the mainline. Variations are skipped by pgn-reader's default.
struct Game {
    white: String,
    black: String,
    result: String,
    opening: Option<String>,
    start: Chess,
    moves: Vec<Move>,
}

#[derive(Default)]
struct Tags {
    white: Option<String>,
    black: Option<String>,
    result: Option<String>,
    opening: Option<String>,
    start: Option<Chess>,
}

struct Moves {
    tags: Tags,
    /// Where the game began: the FEN tag's position, or the standard start.
    start: Chess,
    /// The position after the moves read so far.
    pos: Chess,
    moves: Vec<Move>,
}

struct GameReader;

impl Visitor for GameReader {
    type Tags = Tags;
    type Movetext = Moves;
    type Output = eyre::Result<Game>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(Tags::default())
    }

    fn tag(
        &mut self,
        tags: &mut Self::Tags,
        name: &[u8],
        value: RawTag<'_>,
    ) -> ControlFlow<Self::Output> {
        let text = || value.decode_utf8_lossy().into_owned();
        match name {
            b"White" => tags.white = Some(text()),
            b"Black" => tags.black = Some(text()),
            b"Result" => tags.result = Some(text()),
            b"Opening" => tags.opening = Some(text()),
            b"FEN" => {
                let start = Fen::from_ascii(value.as_bytes())
                    .map_err(|e| eyre!("the FEN tag is not valid FEN: {e}"))
                    .and_then(|f| {
                        f.into_position(CastlingMode::Standard)
                            .map_err(|e| eyre!("the FEN tag is not a legal position: {e}"))
                    });
                match start {
                    Ok(pos) => tags.start = Some(pos),
                    Err(err) => return ControlFlow::Break(Err(err)),
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn begin_movetext(
        &mut self,
        mut tags: Self::Tags,
    ) -> ControlFlow<Self::Output, Self::Movetext> {
        let start = tags.start.take().unwrap_or_default();
        ControlFlow::Continue(Moves {
            tags,
            pos: start.clone(),
            start,
            moves: Vec::new(),
        })
    }

    fn san(
        &mut self,
        movetext: &mut Self::Movetext,
        san_plus: SanPlus,
    ) -> ControlFlow<Self::Output> {
        match san_plus.san.to_move(&movetext.pos) {
            Ok(m) => {
                movetext.pos.play_unchecked(m);
                movetext.moves.push(m);
                ControlFlow::Continue(())
            }
            Err(err) => ControlFlow::Break(Err(eyre!(
                "move {} ({san_plus}) is illegal in the game's position: {err}",
                movetext.moves.len() + 1
            ))),
        }
    }

    fn end_game(&mut self, movetext: Self::Movetext) -> Self::Output {
        let Moves {
            tags, start, moves, ..
        } = movetext;
        Ok(Game {
            white: tags.white.unwrap_or_else(|| "?".into()),
            black: tags.black.unwrap_or_else(|| "?".into()),
            result: tags.result.unwrap_or_else(|| "*".into()),
            opening: tags.opening,
            start,
            moves,
        })
    }
}

fn read_game(pgn: &str) -> eyre::Result<Game> {
    Reader::new(Cursor::new(pgn.as_bytes()))
        .read_game(&mut GameReader)
        .context("read the PGN")?
        .ok_or_else(|| eyre!("no game found in the supplied PGN"))?
}

/// Evaluate every position in a game and report the mistakes.
///
/// One engine process serves the whole review, and every position is
/// searched after `ucinewgame`, which clears the hash table. That preserves
/// the bit-exactness measured on 2026-08-18; reusing a warm table across
/// positions is enough to change node counts, and a review that is not
/// reproducible cannot be compared against another run of itself.
pub async fn review_game(
    pgn: &str,
    engine_path: &Path,
    depth: u32,
    threshold_cp: i32,
    mut on_progress: impl FnMut(usize, usize),
) -> eyre::Result<Review> {
    let game = read_game(pgn)?;
    if game.moves.is_empty() {
        bail!("the supplied PGN contains a game with no moves");
    }
    let root_fen = fen(&game.start);

    let mut engine = Engine::start(engine_path).await?;

    // One pass down the game. Index i describes the position reached after
    // i plies, so index 0 is the starting position.
    let total = game.moves.len() + 1;
    let mut positions: Vec<Chess> = Vec::with_capacity(total);
    let mut evaluations: Vec<i32> = Vec::with_capacity(total);
    let mut lines: Vec<Vec<Move>> = Vec::with_capacity(total);
    let mut played_uci: Vec<String> = Vec::with_capacity(game.moves.len());

    let mut pos = game.start.clone();
    for index in 0..total {
        positions.push(pos.clone());
        if let Some(terminal) = terminal_cp(&pos) {
            evaluations.push(terminal);
            lines.push(Vec::new());
        } else {
            let analysis = engine
                .analyse(&root_fen, &played_uci, depth)
                .await
                .with_context(|| format!("analyse the position after {index} plies"))?;
            evaluations.push(white_cp(analysis.score, pos.turn()));
            lines.push(pv_moves(&pos, &analysis.pv));
        }
        on_progress(index + 1, total);
        if let Some(m) = game.moves.get(index) {
            played_uci.push(UciMove::from_move(*m, CastlingMode::Standard).to_string());
            pos.play_unchecked(*m);
        }
    }
    engine.quit().await?;

    let mut mistakes = Vec::new();
    for (ply, played) in game.moves.iter().enumerate() {
        let (before, after) = (evaluations[ply], evaluations[ply + 1]);
        let mover = positions[ply].turn();
        let clamp = |cp: i32| cp.clamp(-EVAL_CAP_CP, EVAL_CAP_CP);
        // Centipawn loss is stated from the mover's point of view, so a White
        // blunder and a Black blunder are the same positive number.
        let loss = match mover {
            Color::White => clamp(before) - clamp(after),
            Color::Black => clamp(after) - clamp(before),
        };
        if loss < threshold_cp {
            continue;
        }

        let refutation = &lines[ply + 1];
        mistakes.push(Mistake {
            ply,
            move_number: positions[ply].fullmoves().get(),
            side: mover.into(),
            played: san(&positions[ply], played),
            best_line: variation_san(&positions[ply], &lines[ply]),
            best: lines[ply]
                .first()
                .map(|best| san(&positions[ply], best))
                .unwrap_or_default(),
            cp_loss: loss,
            fen_before: fen(&positions[ply]),
            refutation: variation_san(&positions[ply + 1], refutation),
            cost: consequence_of(&positions[ply + 1], refutation),
        });
    }

    Ok(Review {
        white: game.white,
        black: game.black,
        result: game.result,
        opening: game.opening,
        depth,
        threshold_cp,
        mistake_count: mistakes.len(),
        mistakes,
        evaluations,
    })
}

/// The engine's PV as legal moves, stopping at the first one that does not
/// parse or is illegal. Neither should happen with Stockfish, and truncating
/// keeps a bad line from producing a material diff over a corrupted board.
fn pv_moves(pos: &Chess, pv: &[String]) -> Vec<Move> {
    let mut walk = pos.clone();
    let mut out = Vec::new();
    for text in pv {
        let Ok(m) = UciMove::from_ascii(text.as_bytes())
            .map_err(|_| ())
            .and_then(|u| u.to_move(&walk).map_err(|_| ()))
        else {
            break;
        };
        walk.play_unchecked(m);
        out.push(m);
    }
    out
}

fn variation_san(pos: &Chess, pv: &[Move]) -> Vec<String> {
    let mut walk = pos.clone();
    pv.iter()
        .map(|m| SanPlus::from_move_and_play_unchecked(&mut walk, *m).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consequence(fen_text: &str, pv_san: &[&str]) -> Cost {
        let pos: Chess = Fen::from_ascii(fen_text.as_bytes())
            .expect("test FEN parses")
            .into_position(CastlingMode::Standard)
            .expect("test FEN is legal");
        let mut walk = pos.clone();
        let pv: Vec<Move> = pv_san
            .iter()
            .map(|text| {
                let m = SanPlus::from_ascii(text.as_bytes())
                    .expect("test SAN parses")
                    .san
                    .to_move(&walk)
                    .expect("test SAN is legal");
                walk.play_unchecked(m);
                m
            })
            .collect();
        consequence_of(&pos, &pv)
    }

    #[test]
    fn material_won_is_counted_from_the_mistaker_side() {
        // Black to move with a knight on b3 and White's rook undefended on a1.
        // (The Python suite had the knight on c2, where it also checks the
        // White king with Black to move -- an illegal position python-chess
        // accepted and shakmaty rejects.)
        let c = consequence("4k3/8/8/8/8/1n6/8/R3K3 b - - 0 1", &["Nxa1"]);
        assert_eq!(c.kind, CostKind::Material);
        assert_eq!(c.material_swing, 5);
        assert_eq!(c.squares, ["a1"]);
    }

    #[test]
    fn back_rank_mate_is_its_own_kind() {
        let c = consequence("r5k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1", &["Ra1#"]);
        assert_eq!(c.kind, CostKind::Mate);
        assert_eq!(c.mate_in, Some(1));
    }

    #[test]
    fn an_even_trade_is_not_reported_as_winning_material() {
        // Rooks come off on d8. Material changes hands twice and nets to
        // nothing, so narrating the captures as the cost would be false.
        let c = consequence("3rk3/8/8/8/8/8/8/3RK3 w - - 0 1", &["Rxd8+", "Kxd8"]);
        assert_eq!(c.kind, CostKind::TradeNeutral);
        assert_eq!(c.material_swing, 0);
        assert_eq!(c.squares, ["d8", "d8"]);
    }

    #[test]
    fn a_quiet_refutation_reports_nothing_rather_than_guessing() {
        let c = consequence(
            "r1bqkbnr/pppppppp/2n5/8/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 1",
            &["Bb5", "a6", "Ba4"],
        );
        assert_eq!(c.kind, CostKind::Positional);
        assert_eq!(c.material_swing, 0);
        assert!(c.squares.is_empty());
    }

    #[test]
    fn a_delivered_mate_is_scored_for_the_winner() {
        // The bug this guards: a mated position read as a 2000cp blunder by
        // the side that delivered mate. White has just mated; the position
        // must score as a White win, not hand a sign-less `mate 0` around.
        let mated: Chess =
            Fen::from_ascii(b"r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4")
                .expect("FEN parses")
                .into_position(CastlingMode::Standard)
                .expect("FEN is legal");
        assert_eq!(terminal_cp(&mated), Some(MATE_CP));
    }

    #[test]
    fn engine_scores_are_turned_to_whites_point_of_view() {
        assert_eq!(white_cp(Score::Cp(50), Color::White), 50);
        assert_eq!(white_cp(Score::Cp(50), Color::Black), -50);
        assert_eq!(white_cp(Score::Mate(2), Color::Black), -MATE_CP);
        assert_eq!(white_cp(Score::Mate(-1), Color::Black), MATE_CP);
    }

    #[test]
    fn a_pgn_with_no_moves_parses_to_an_empty_game() {
        let game = read_game("[Event \"empty\"]\n\n*\n").expect("an empty game parses");
        assert!(game.moves.is_empty());
    }

    /// Stockfish-dependent tests. They check bookkeeping (which side erred,
    /// that the loss is positive) rather than the engine's judgement, so a
    /// shallow depth is enough.
    mod with_engine {
        use super::super::*;
        use crate::engine_path;

        // 3... Nf6 walks into Qxf7#. A Black mistake, which is where a sign
        // error in the loss calculation would show up as a negative loss or as
        // the blunder being attributed to White.
        const SCHOLARS_MATE: &str = "[Event \"test\"]\n[White \"w\"]\n[Black \"b\"]\n\
            [Result \"1-0\"]\n\n1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7# 1-0\n";

        #[tokio::test]
        async fn a_black_mistake_is_attributed_to_black_with_a_positive_loss() {
            let Ok(path) = engine_path() else { return };
            let review = review_game(SCHOLARS_MATE, &path, 8, 200, |_, _| {})
                .await
                .expect("review runs");
            assert_eq!(review.result, "1-0");
            let blunder = review
                .mistakes
                .iter()
                .find(|m| m.played == "Nf6")
                .expect("Nf6 is flagged");
            assert_eq!(blunder.side, Side::Black);
            assert!(blunder.cp_loss > 0);
            assert_eq!(blunder.move_number, 3);
            assert_eq!(blunder.ply, 5);
            // The winning move itself must not be flagged as a White mistake.
            assert!(review.mistakes.iter().all(|m| m.played != "Qxf7#"));
            // One evaluation per position, start included, ending on the
            // delivered mate scored for White.
            assert_eq!(review.evaluations.len(), 8);
            assert_eq!(review.evaluations.last(), Some(&MATE_CP));
            assert_eq!(blunder.best_line.first(), Some(&blunder.best));
        }

        #[tokio::test]
        async fn progress_is_reported_once_per_position() {
            let Ok(path) = engine_path() else { return };
            let mut seen = Vec::new();
            review_game(SCHOLARS_MATE, &path, 6, 200, |done, total| {
                seen.push((done, total))
            })
            .await
            .expect("review runs");
            // Seven plies, so eight positions including the start.
            assert_eq!(seen, (1..=8).map(|d| (d, 8)).collect::<Vec<_>>());
        }

        #[tokio::test]
        async fn a_pgn_with_no_moves_is_rejected_at_the_boundary() {
            let Ok(path) = engine_path() else { return };
            let err = review_game("[Event \"empty\"]\n\n*\n", &path, 6, 200, |_, _| {})
                .await
                .expect_err("no moves is an error");
            assert!(err.to_string().contains("no moves"));
        }
    }
}
