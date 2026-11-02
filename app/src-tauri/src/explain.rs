//! Why a mistake's refutation works, as facts about the board.
//!
//! A review already says *what* the refutation takes ([`Cost`]). This module
//! says *why it was available*: which piece was loose and what your move had
//! to do with it, which move forked two things, which piece was pinned.
//!
//! [`Cost`]: crate::analysis::Cost
//!
//! # How it works
//!
//! Everything is computed by playing moves on the board, never estimated, so
//! that every fact is true of the position it names:
//!
//! - **Exchanges are played with legal moves.** [`exchange`] tries every legal
//!   capture on a square and every legal recapture after it, and lets either
//!   side stop when continuing would lose. A pinned piece cannot recapture and
//!   a side in check must answer the check, because the move generator says
//!   so -- not because the evaluator remembered to ask.
//! - **The whole refutation line is read, not only its first move.** The
//!   engine's line already shows how the material is won. The explanation
//!   finds the move in that line that wins material, then walks backwards to
//!   the move that made it possible: yours, a forced reply of yours later in
//!   the line, or one of your opponent's.
//!
//! Positions are numbered by ply into the refutation line:
//!
//! ```text
//!   before --your move--> P0 --ref[0]--> P1 --ref[1]--> P2 ...
//!                          ^ opponent to move     ^ opponent to move again
//! ```
//!
//! A fact's `at_ply` is the index into `ref` of the move it describes, so
//! `at_ply: 0` is the opponent's reply to your mistake. Even plies are your
//! opponent's moves, odd plies yours.
//!
//! The four kinds of fact, and the account's mistakes that motivated each
//! (`tests/fixtures/explain-cases.json`, from 8 reviewed games, 2026-11-01):
//!
//! - **Loose** -- the move that wins material captures on a square where the
//!   exchange favours the capturer. 2.g4 Bxg4: you moved the pawn there.
//! - **Fork** -- one move attacks two or more things worth taking, and the
//!   line collects one. 5...Nxe4 Qa4+: check, and the knight on e4 hangs.
//! - **Pin** -- a move pins one of your pieces, and the pin is what the loss
//!   turns on. 5.b3 Bb4: the knight on c3 cannot move, Bd2 to cover it leaves
//!   d4 without its defenders, and Nxd4 follows.
//! - **Mate** is already reported by `CostKind::Mate` and gets no fact here.
//!
//! When none of these fits, the result is empty. That is a gap in coverage,
//! not an accepted outcome: every position in the fixture must be explained,
//! and one that is not fails the test.

use serde::{Deserialize, Serialize};
use shakmaty::attacks;
use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::{Bitboard, Board, CastlingMode, Chess, Color, Move, Position, Role, Square};

use crate::analysis::{CostKind, Mistake, Review};

/// A piece and where it stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub role: String,
    pub square: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub role: String,
    pub square: String,
    /// The target is the king: the forking move gives check.
    pub check: bool,
}

/// What made the lost piece loose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LooseCause {
    /// Your move put the piece on the square where it is lost.
    MovedThere,
    /// The piece was already loose before your move, which left it so.
    Ignored,
    /// Your move took away a piece that was defending the square.
    DefenderMoved { from: String, to: String },
    /// Your move took a piece out of a line, letting an attacker reach the
    /// square or blocking a defender.
    LineOpened { from: String },
    /// An opponent's move later in the line attacked the square once more
    /// than it was defended.
    AttackerAdded { at_ply: usize, by: Unit },
    /// An opponent's move pinned a defender, so it can no longer recapture.
    DefenderPinned {
        at_ply: usize,
        pinned: Unit,
        by: Unit,
    },
    /// One of your moves later in the line -- usually a forced recapture or
    /// a reply to a check or pin -- took defenders away from the square.
    DefendersLost {
        at_ply: usize,
        moved: Unit,
        from: String,
        lost: Vec<Unit>,
    },
    /// A move of yours later in the line put the piece there, and it was
    /// loose on arrival (typically a recapture).
    ArrivedLoose { at_ply: usize },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    Loose {
        at_ply: usize,
        piece: Unit,
        /// Opponent pieces that can capture on the square, cheapest first.
        attackers: Vec<Unit>,
        /// Your pieces that can legally recapture there, cheapest first.
        defenders: Vec<Unit>,
        /// Pawns the opponent comes out ahead if the exchange is played out.
        wins: i32,
        /// Nested, not flattened: several causes carry their own `at_ply`,
        /// which would collide with the capture's.
        cause: LooseCause,
    },
    Fork {
        at_ply: usize,
        by: Unit,
        targets: Vec<Target>,
    },
    Pin {
        at_ply: usize,
        by: Unit,
        pinned: Unit,
        /// The piece behind: the king for an absolute pin, otherwise one
        /// worth more than the pinned piece.
        behind: Unit,
    },
}

/// Exchange values in pawns.
fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        // Never captured: the legal move generator does not allow it.
        Role::King => 100,
    }
}

fn role_name(role: Role) -> String {
    format!("{role:?}").to_lowercase()
}

fn unit(board: &Board, square: Square) -> Option<Unit> {
    board.piece_at(square).map(|piece| Unit {
        role: role_name(piece.role),
        square: square.to_string(),
    })
}

/// What the side to move gains, in pawns, from the exchange on `square`,
/// trying every legal capture there and every legal recapture, either side
/// free to stop. Zero when it has no capture worth making.
///
/// A plain search rather than the usual static exchange evaluation, which
/// counts attackers by geometry and so counts pinned pieces and ignores
/// checks; on these positions that produced false defenders. Only captures
/// on one square are searched, and each removes a piece, so the depth is
/// bounded by the pieces bearing on the square.
pub fn exchange(pos: &Chess, square: Square) -> i32 {
    let mut best = 0;
    for m in pos.legal_moves() {
        if m.to() != square || matches!(m, Move::EnPassant { .. }) {
            continue;
        }
        let Some(captured) = m.capture() else {
            continue;
        };
        // Promote to a queen only; under-promotion never wins more here.
        if m.promotion().is_some_and(|role| role != Role::Queen) {
            continue;
        }
        let promoted = m
            .promotion()
            .map_or(0, |role| value(role) - value(Role::Pawn));
        let mut next = pos.clone();
        next.play_unchecked(m);
        best = best.max(value(captured) + promoted - exchange(&next, square));
    }
    best
}

/// [`exchange`] for `side` whether or not it is its turn: if it is the other
/// side's move, pass the move first. `None` when that is impossible because
/// the side to move is in check, so "what if it were the other side's turn"
/// has no answer.
fn exchange_for(pos: &Chess, square: Square, side: Color) -> Option<i32> {
    if pos.turn() == side {
        return Some(exchange(pos, square));
    }
    let passed = pos.clone().swap_turn().ok()?;
    Some(exchange(&passed, square))
}

/// The opponent slider pinning the piece on `square` to its own king, if any.
fn absolute_pinner(board: &Board, square: Square) -> Option<Square> {
    let piece = board.piece_at(square)?;
    let king = board.king_of(piece.color)?;
    pinner_between(board, square, king, !piece.color)
}

/// An enemy slider on the far side of `square` from `behind`, on a line it
/// moves along, with nothing else between.
fn pinner_between(board: &Board, square: Square, behind: Square, enemy: Color) -> Option<Square> {
    // `ray` is the whole rank, file or diagonal through both squares, empty
    // when they share none.
    if square == behind || attacks::ray(behind, square).is_empty() {
        return None;
    }
    let diagonal = behind.file() != square.file() && behind.rank() != square.rank();
    let sliders = if diagonal {
        board.bishops_and_queens()
    } else {
        board.rooks_and_queens()
    } & board.by_color(enemy);
    // `between` is empty unless its ends are aligned, so containing `square`
    // also means the slider is on the same line, beyond it.
    sliders.into_iter().find(|&slider| {
        let gap = attacks::between(behind, slider);
        gap.contains(square) && (gap & board.occupied()) == Bitboard::from(square)
    })
}

/// Pieces of `side` that can capture on `square`, cheapest first: those
/// attacking it and not pinned off the line to it. The king counts only
/// when the square has a single enemy attacker, since otherwise recapturing
/// would walk into check.
fn capturers(board: &Board, square: Square, side: Color) -> Vec<Unit> {
    let attackers = board.attacks_to(square, side, board.occupied()) & board.occupied();
    let enemy_attackers =
        (board.attacks_to(square, !side, board.occupied()) & board.occupied()).count();
    let mut units: Vec<(i32, Unit)> = attackers
        .into_iter()
        .filter(|&from| {
            let Some(piece) = board.piece_at(from) else {
                return false;
            };
            if piece.role == Role::King {
                return enemy_attackers <= 1;
            }
            match absolute_pinner(board, from) {
                None => true,
                Some(pinner) => {
                    let king = board.king_of(side).expect("a side with pieces has a king");
                    attacks::aligned(king, pinner, square)
                }
            }
        })
        .filter_map(|from| {
            let piece = board.piece_at(from)?;
            Some((value(piece.role), unit(board, from)?))
        })
        .collect();
    units.sort_by_key(|(v, _)| *v);
    units.into_iter().map(|(_, u)| u).collect()
}

/// Facts about why `refutation` punishes `played` from `before`.
pub fn explain(before: &Chess, played: Move, refutation: &[Move]) -> Vec<Fact> {
    let mistaker = before.turn();
    let opponent = !mistaker;

    // positions[i] is the position before ref[i]; positions[0] is right after
    // the mistake. One more position than moves.
    let mut positions = Vec::with_capacity(refutation.len() + 1);
    let mut walk = before.clone();
    walk.play_unchecked(played);
    positions.push(walk.clone());
    for &m in refutation {
        walk.play_unchecked(m);
        positions.push(walk.clone());
    }

    let mut facts = Vec::new();
    // The decisive move: the first capture by the opponent where the exchange
    // on that square favours them. Everything is explained relative to it.
    let decisive = (0..refutation.len()).step_by(2).find(|&ply| {
        let m = refutation[ply];
        m.is_capture()
            && !matches!(m, Move::EnPassant { .. })
            && exchange(&positions[ply], m.to()) > 0
    });
    // A fork counts only if it comes no later than the decisive capture. One
    // further down the engine's line is something that could happen after
    // the material is already won, not a reason it was lost.
    let fork = find_fork(&positions, refutation, mistaker).filter(
        |f| matches!(f, Fact::Fork { at_ply, .. } if decisive.is_none_or(|d| *at_ply <= d)),
    );

    if let Some(ply) = decisive {
        let square = refutation[ply].to();
        let covered_by_fork = matches!(&fork, Some(Fact::Fork { targets, .. })
            if targets.iter().any(|t| t.square == square.to_string()));
        if !covered_by_fork {
            let cause = loose_cause(
                before, played, &positions, refutation, ply, square, opponent,
            );
            if let Some(LooseCause::DefendersLost { at_ply, .. }) = &cause
                && let Some(pin) = pin_before(&positions, refutation, *at_ply, mistaker)
            {
                facts.push(pin);
            }
            if let (Some(cause), Some(piece)) = (cause, unit(positions[ply].board(), square)) {
                facts.push(Fact::Loose {
                    at_ply: ply,
                    piece,
                    attackers: capturers(positions[ply].board(), square, opponent),
                    defenders: capturers(positions[ply].board(), square, mistaker),
                    wins: exchange(&positions[ply], square),
                    cause,
                });
            }
        }
    }
    if let Some(fork) = fork {
        facts.insert(0, fork);
    }
    facts
}

/// The first opponent move in the line that attacks two or more of your
/// units worth taking, where the line then captures one of them.
fn find_fork(positions: &[Chess], refutation: &[Move], mistaker: Color) -> Option<Fact> {
    let opponent = !mistaker;
    for ply in (0..refutation.len()).step_by(2) {
        let landing = refutation[ply].to();
        let after = &positions[ply + 1];
        let attacker = after.board().piece_at(landing)?;
        let targets: Vec<Target> = (after.board().attacks_from(landing)
            & after.board().by_color(mistaker))
        .into_iter()
        .filter_map(|square| {
            let piece = after.board().piece_at(square)?;
            let check = piece.role == Role::King;
            let worth = check
                || value(piece.role) > value(attacker.role)
                || en_prise(positions, ply, square, opponent);
            worth.then(|| Target {
                role: role_name(piece.role),
                square: square.to_string(),
                check,
            })
        })
        .collect();
        let collected = refutation
            .iter()
            .enumerate()
            .skip(ply + 2)
            .step_by(2)
            .any(|(_, m)| m.is_capture() && targets.iter().any(|t| t.square == m.to().to_string()));
        if targets.len() >= 2 && collected {
            return Some(Fact::Fork {
                at_ply: ply,
                by: Unit {
                    role: role_name(attacker.role),
                    square: landing.to_string(),
                },
                targets,
            });
        }
    }
    None
}

/// Whether the opponent could profitably take on `square` after its move at
/// `ply`. Asked as "if it were the opponent's turn again" -- unless you are
/// in check, when that question has no answer; then it is asked of the
/// position after your actual reply in the line, which is when the opponent
/// does get to take.
fn en_prise(positions: &[Chess], ply: usize, square: Square, opponent: Color) -> bool {
    let after = &positions[ply + 1];
    if let Some(gain) = exchange_for(after, square, opponent) {
        return gain > 0;
    }
    positions
        .get(ply + 2)
        .is_some_and(|next| next.turn() == opponent && exchange(next, square) > 0)
}

/// Why the piece on `square` could be won at `ply`.
#[allow(clippy::too_many_arguments)]
fn loose_cause(
    before: &Chess,
    played: Move,
    positions: &[Chess],
    refutation: &[Move],
    ply: usize,
    square: Square,
    opponent: Color,
) -> Option<LooseCause> {
    let mistaker = !opponent;
    // When the piece now on the square arrived: your mistake, or a move of
    // yours in the line (counted as its ply), or before either.
    let arrived_in_line = (0..ply)
        .rev()
        .find(|&i| i % 2 == 1 && refutation[i].to() == square);
    if arrived_in_line.is_none() && played.to() == square {
        return Some(LooseCause::MovedThere);
    }

    // Looseness at each position from the piece's arrival to the capture,
    // asked as "could the opponent win it if it were their move". Index -1
    // stands for the position before the mistake.
    let first = arrived_in_line.map_or(-1, |i| i as isize + 1);
    let loose_at = |index: isize| -> Option<bool> {
        let pos = if index < 0 {
            before
        } else {
            &positions[index as usize]
        };
        let occupant = pos.board().piece_at(square)?;
        if occupant.color != mistaker {
            return None;
        }
        exchange_for(pos, square, opponent).map(|gain| gain > 0)
    };
    let last_safe = (first..=ply as isize)
        .rev()
        .find(|&i| loose_at(i) == Some(false));

    let Some(safe) = last_safe else {
        return Some(match arrived_in_line {
            Some(at_ply) => LooseCause::ArrivedLoose { at_ply },
            None => LooseCause::Ignored,
        });
    };

    // The moves between the last safe position and the next one where it is
    // loose; the first of them that changes the square's defence is the
    // cause. Index -1 to 0 is your mistake.
    let next_loose = ((safe + 1)..=ply as isize).find(|&i| loose_at(i) == Some(true))?;
    for index in safe..next_loose {
        let (from_pos, to_pos, mover_move) = if index < 0 {
            (before, &positions[0], played)
        } else {
            let i = index as usize;
            (&positions[i], &positions[i + 1], refutation[i])
        };
        let defenders_before = capturers(from_pos.board(), square, mistaker);
        let defenders_after = capturers(to_pos.board(), square, mistaker);
        let attackers_before = capturers(from_pos.board(), square, opponent);
        let attackers_after = capturers(to_pos.board(), square, opponent);
        let moved_by_opponent = from_pos.turn() == opponent;
        let from = mover_move
            .from()
            .expect("a played move has an origin square");

        if index < 0 {
            // Your mistake made it loose.
            return Some(if before.board().attacks_from(from).contains(square) {
                LooseCause::DefenderMoved {
                    from: from.to_string(),
                    to: mover_move.to().to_string(),
                }
            } else {
                LooseCause::LineOpened {
                    from: from.to_string(),
                }
            });
        }
        let at_ply = index as usize;
        if moved_by_opponent {
            // A defender still on the board but no longer able to recapture
            // has been pinned by this move.
            if let Some(pinned) = defenders_before.iter().find(|d| {
                !defenders_after.contains(d)
                    && to_pos
                        .board()
                        .piece_at(d.square.parse().expect("units hold valid squares"))
                        .is_some_and(|p| p.color == mistaker)
            }) && let Some(by) = unit(to_pos.board(), mover_move.to())
            {
                return Some(LooseCause::DefenderPinned {
                    at_ply,
                    pinned: pinned.clone(),
                    by,
                });
            }
            if attackers_after.len() > attackers_before.len() {
                let by = attackers_after
                    .iter()
                    .find(|a| !attackers_before.contains(a))
                    .cloned()
                    .or_else(|| unit(to_pos.board(), mover_move.to()))?;
                return Some(LooseCause::AttackerAdded { at_ply, by });
            }
        } else {
            let lost: Vec<Unit> = defenders_before
                .iter()
                .filter(|d| !defenders_after.contains(d))
                .cloned()
                .collect();
            if !lost.is_empty() {
                return Some(LooseCause::DefendersLost {
                    at_ply,
                    moved: unit(to_pos.board(), mover_move.to())?,
                    from: from.to_string(),
                    lost,
                });
            }
        }
    }
    None
}

/// A pin made by the opponent's move just before `at_ply`, on a piece of
/// yours, when your move at `at_ply` is the one that gave up the defence --
/// that is, the pin is why you had to make it.
fn pin_before(
    positions: &[Chess],
    refutation: &[Move],
    at_ply: usize,
    mistaker: Color,
) -> Option<Fact> {
    let ply = at_ply.checked_sub(1)?;
    let pinning = refutation[ply];
    let board = positions[ply + 1].board();
    let by_square = pinning.to();
    let by = board.piece_at(by_square)?;
    if !matches!(by.role, Role::Bishop | Role::Rook | Role::Queen) {
        return None;
    }
    // Your pieces on a line from the pinning piece, with exactly one piece
    // between it and something worth more (or the king).
    for behind in board.by_color(mistaker) {
        let behind_piece = board.piece_at(behind)?;
        // Exactly one piece between the pinner and the piece behind. An
        // empty `between` means the two are not on a common line.
        let between = attacks::between(by_square, behind) & board.occupied();
        if between.count() != 1 {
            continue;
        }
        let pinned_square = between.into_iter().next()?;
        let Some(pinned_piece) = board.piece_at(pinned_square) else {
            continue;
        };
        // The pinner must move along that line: a bishop pins on diagonals
        // only, a rook on ranks and files. It attacks the pinned piece
        // exactly when it does.
        let slides_that_way = board.attacks_from(by_square).contains(pinned_square);
        let worth_it =
            behind_piece.role == Role::King || value(behind_piece.role) > value(pinned_piece.role);
        if pinned_piece.color == mistaker && slides_that_way && worth_it {
            return Some(Fact::Pin {
                at_ply: ply,
                by: unit(board, by_square)?,
                pinned: unit(board, pinned_square)?,
                behind: unit(board, behind)?,
            });
        }
    }
    None
}

/// [`explain`] for a stored mistake, parsing its position and moves. Mates
/// get no fact; `CostKind::Mate` already says what happens. Empty if the
/// stored position or moves fail to parse, which would mean a corrupt review.
pub fn explain_mistake(m: &Mistake) -> Vec<Fact> {
    if m.cost.kind == CostKind::Mate {
        return Vec::new();
    }
    let parse = || -> Option<Vec<Fact>> {
        let before: Chess = Fen::from_ascii(m.fen_before.as_bytes())
            .ok()?
            .into_position(CastlingMode::Standard)
            .ok()?;
        let played = San::from_ascii(trim(&m.played))
            .ok()?
            .to_move(&before)
            .ok()?;
        let mut walk = before.clone();
        walk.play_unchecked(played);
        let mut refutation = Vec::new();
        for text in &m.refutation {
            let Some(next) = San::from_ascii(trim(text))
                .ok()
                .and_then(|san| san.to_move(&walk).ok())
            else {
                break;
            };
            walk.play_unchecked(next);
            refutation.push(next);
        }
        Some(explain(&before, played, &refutation))
    };
    parse().unwrap_or_default()
}

/// `review` with every mistake's facts worked out afresh. Called on every
/// review handed to the webview, stored or new.
pub fn with_facts(mut review: Review) -> Review {
    for m in &mut review.mistakes {
        m.facts = explain_mistake(m);
    }
    review
}

/// SAN without its check or mate suffix, which `San::from_ascii` rejects.
fn trim(san: &str) -> &[u8] {
    san.trim_end_matches(['+', '#']).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Cost, Severity, Side};

    /// One line per fact, compact enough to compare and to read:
    /// `fork@0 a4 e8+ e4`, `pin@0 b4>c3>e1`,
    /// `loose@2 d4 defenders_lost@1`, `loose@0 g4 moved_there`.
    fn summary(facts: &[Fact]) -> Vec<String> {
        facts
            .iter()
            .map(|fact| match fact {
                Fact::Fork {
                    at_ply,
                    by,
                    targets,
                } => {
                    let targets: Vec<String> = targets
                        .iter()
                        .map(|t| format!("{}{}", t.square, if t.check { "+" } else { "" }))
                        .collect();
                    format!("fork@{at_ply} {} {}", by.square, targets.join(" "))
                }
                Fact::Pin {
                    at_ply,
                    by,
                    pinned,
                    behind,
                } => {
                    format!(
                        "pin@{at_ply} {}>{}>{}",
                        by.square, pinned.square, behind.square
                    )
                }
                Fact::Loose {
                    at_ply,
                    piece,
                    cause,
                    ..
                } => {
                    let cause = match cause {
                        LooseCause::MovedThere => "moved_there".to_owned(),
                        LooseCause::Ignored => "ignored".to_owned(),
                        LooseCause::DefenderMoved { .. } => "defender_moved".to_owned(),
                        LooseCause::LineOpened { .. } => "line_opened".to_owned(),
                        LooseCause::AttackerAdded { at_ply, .. } => {
                            format!("attacker_added@{at_ply}")
                        }
                        LooseCause::DefenderPinned { at_ply, .. } => {
                            format!("defender_pinned@{at_ply}")
                        }
                        LooseCause::DefendersLost { at_ply, .. } => {
                            format!("defenders_lost@{at_ply}")
                        }
                        LooseCause::ArrivedLoose { at_ply } => format!("arrived_loose@{at_ply}"),
                    };
                    format!("loose@{at_ply} {} {cause}", piece.square)
                }
            })
            .collect()
    }

    #[derive(serde::Deserialize)]
    struct Case {
        fen_before: String,
        played: String,
        refutation: Vec<String>,
        kind: String,
        expect: Vec<String>,
    }

    fn mistake(case: &Case) -> Mistake {
        Mistake {
            ply: 0,
            move_number: 0,
            side: Side::White,
            played: case.played.clone(),
            best: String::new(),
            best_line: Vec::new(),
            severity: Severity::Mistake,
            chances_lost: 0.0,
            cp_loss: 0,
            fen_before: case.fen_before.clone(),
            refutation: case.refutation.clone(),
            cost: Cost {
                material_swing: 0,
                squares: Vec::new(),
                mate_in: None,
                kind: if case.kind == "mate" {
                    CostKind::Mate
                } else {
                    CostKind::Material
                },
            },
            facts: Vec::new(),
        }
    }

    /// Every drillable mistake from the account's reviewed games gets the
    /// explanation recorded for it. A position the detectors cannot explain
    /// is a failure here, not a known limit.
    #[test]
    fn every_real_mistake_is_explained() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/explain-cases.json"))
                .expect("the fixture parses");
        let mut wrong = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            let got = summary(&explain_mistake(&mistake(case)));
            println!(
                "{index:>2} {:<8} {:<7} {}  =>  {got:?}",
                case.kind,
                case.played,
                case.refutation
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let unexplained = case.kind != "mate" && got.is_empty();
            if got != case.expect || unexplained {
                wrong.push(index);
            }
        }
        assert!(
            wrong.is_empty(),
            "cases explained wrongly or not at all: {wrong:?}"
        );
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;

    fn position(fen: &str) -> Chess {
        Fen::from_ascii(fen.as_bytes())
            .expect("test FEN parses")
            .into_position(CastlingMode::Standard)
            .expect("test FEN is legal")
    }

    /// The case static exchange evaluation gets wrong: the knight on e2
    /// "defends" d4 by geometry but is pinned to its king by the rook on e8,
    /// so Nxd4 wins the bishop outright.
    #[test]
    fn a_pinned_defender_cannot_recapture() {
        let d4: Square = "d4".parse().expect("square");
        let pinned = position("4r1k1/8/2n5/8/3B4/8/4N3/4K3 b - - 0 1");
        assert_eq!(exchange(&pinned, d4), 3);
        assert!(capturers(pinned.board(), d4, Color::White).is_empty());

        // The same with the rook off the e-file: the knight recaptures.
        let free = position("r5k1/8/2n5/8/3B4/8/4N3/4K3 b - - 0 1");
        assert_eq!(exchange(&free, d4), 0);
        assert_eq!(capturers(free.board(), d4, Color::White).len(), 1);
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    /// The webview reads the capture's ply and the cause's ply separately.
    /// When the cause was flattened into the fact, both were serialised as
    /// one `at_ply` key and the cause's overwrote the capture's, so the app
    /// named the wrong move as the one that wins the piece.
    #[test]
    fn a_loose_fact_keeps_the_capture_ply_and_the_cause_ply_apart() {
        let pawn = Unit {
            role: "pawn".into(),
            square: "e3".into(),
        };
        let queen = Unit {
            role: "queen".into(),
            square: "g5".into(),
        };
        let fact = Fact::Loose {
            at_ply: 6,
            piece: pawn,
            attackers: vec![queen.clone()],
            defenders: Vec::new(),
            wins: 1,
            cause: LooseCause::AttackerAdded {
                at_ply: 4,
                by: queen,
            },
        };
        let json = serde_json::to_value(&fact).expect("serialises");
        assert_eq!(json["at_ply"], 6);
        assert_eq!(json["cause"]["at_ply"], 4);
        assert_eq!(json["cause"]["type"], "attacker_added");
    }
}
