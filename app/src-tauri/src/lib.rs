//! Chess Coach: reviews a player's chess.com games and says why each mistake
//! was a mistake.
//!
//! The webview (`app/src`) calls the commands below through Tauri's IPC. They
//! sit over three modules:
//!
//! ```text
//!   chesscom.rs  -- import: fetch monthly archives from api.chess.com
//!   store.rs     -- SQLite: games, fetched archives, finished reviews
//!   analysis.rs  -- review: Stockfish over every position (via uci.rs),
//!                   then what each mistake's refutation takes
//!   practice.rs  -- the trainer's spaced schedule over past mistakes
//! ```
//!
//! Errors cross the IPC boundary as strings with the full `eyre` context
//! chain, because the only consumer is a person reading them in the window.

mod analysis;
mod chesscom;
mod explain;
mod practice;
mod store;
mod uci;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Context, eyre};
use serde::Serialize;
use tauri::Manager;
use tauri::ipc::Channel;

use crate::analysis::{CostKind, Mistake, MoveJudgement, Review, Side};
use crate::practice::Grade;
use crate::store::{GameDetail, GameSummary, Store};

/// Default search depth. 16 keeps a 60-ply review near half a minute while
/// staying far past the horizon of the mistakes this is meant to find.
const DEFAULT_DEPTH: u32 = 16;

struct AppState {
    /// `std::sync::Mutex`, not tokio's: every lock is taken and released
    /// between awaits, never held across one, and queries are local and fast.
    store: Mutex<Store>,
    engine: PathBuf,
}

type CommandResult<T> = Result<T, String>;

fn report(err: eyre::Report) -> String {
    format!("{err:#}")
}

/// Path to Stockfish.
///
/// `PATH` alone is not enough: a macOS app launched from Finder or the Dock
/// inherits launchd's minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which
/// has no Homebrew directory. So the Homebrew prefixes are searched after it.
pub(crate) fn engine_path() -> eyre::Result<PathBuf> {
    let from_env = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let homebrew = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    from_env
        .into_iter()
        .chain(homebrew)
        .map(|dir| dir.join("stockfish"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            eyre!(
                "stockfish was not found on PATH or in /opt/homebrew/bin or /usr/local/bin; \
                 install it with `brew install stockfish`"
            )
        })
}

#[derive(Serialize)]
struct ImportResult {
    username: String,
    archives_fetched: usize,
    archives_skipped: usize,
    games_new: usize,
}

#[derive(Serialize)]
struct GamePage {
    total: i64,
    offset: i64,
    games: Vec<GameSummary>,
}

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
}

/// The chess.com account this app pulls games for, or `None` before the
/// first-run screen has been completed.
#[tauri::command]
fn account(state: tauri::State<'_, AppState>) -> CommandResult<Option<String>> {
    state
        .store
        .lock()
        .expect("store lock poisoned")
        .setting(ACCOUNT_KEY)
        .map_err(report)
}

/// First run: import `username`'s games, and remember the account only once
/// that succeeds. A mistyped name fails the import (chess.com answers 404),
/// so it is never saved and the first-run screen stays up with the error.
#[tauri::command]
async fn connect_account(
    state: tauri::State<'_, AppState>,
    username: String,
) -> CommandResult<ImportResult> {
    let username = username.trim().to_owned();
    if username.is_empty() {
        return Err("enter a chess.com username".into());
    }
    let result = import_for(&state, &username).await.map_err(report)?;
    state
        .store
        .lock()
        .expect("store lock poisoned")
        .set_setting(ACCOUNT_KEY, &username)
        .map_err(report)?;
    Ok(result)
}

/// Every later launch: pull new games for the remembered account.
#[tauri::command]
async fn sync_games(state: tauri::State<'_, AppState>) -> CommandResult<ImportResult> {
    let username = state
        .store
        .lock()
        .expect("store lock poisoned")
        .setting(ACCOUNT_KEY)
        .map_err(report)?
        .ok_or("no chess.com account has been connected yet")?;
    import_for(&state, &username).await.map_err(report)
}

const ACCOUNT_KEY: &str = "chesscom_username";

/// Import a chess.com player's finished games: every monthly archive not
/// fetched before, plus the newest, which keeps gaining games until it ends.
async fn import_for(state: &AppState, username: &str) -> eyre::Result<ImportResult> {
    let username = username.to_owned();
    let run = async {
        let http = chesscom::client()?;
        let urls = chesscom::archive_urls(&http, &username).await?;
        let fetched = state
            .store
            .lock()
            .expect("store lock poisoned")
            .fetched_archives()?;
        let to_fetch = chesscom::archives_to_fetch(&urls, &fetched);
        let mut new = 0;
        for url in &to_fetch {
            let games = chesscom::archive_games(&http, url).await?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("read the clock")?
                .as_secs();
            let mut store = state.store.lock().expect("store lock poisoned");
            new += store.upsert_games(&username, &games)?;
            store.mark_fetched(url, &username, now as i64)?;
        }
        Ok(ImportResult {
            archives_fetched: to_fetch.len(),
            archives_skipped: urls.len() - to_fetch.len(),
            games_new: new,
            username: username.clone(),
        })
    };
    run.await
}

#[tauri::command]
fn list_games(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> CommandResult<GamePage> {
    let (limit, offset) = (
        limit.unwrap_or(200).clamp(1, 500),
        offset.unwrap_or(0).max(0),
    );
    let store = state.store.lock().expect("store lock poisoned");
    Ok(GamePage {
        total: store.count_games().map_err(report)?,
        offset,
        games: store.list_games(limit, offset).map_err(report)?,
    })
}

#[tauri::command]
fn get_game(state: tauri::State<'_, AppState>, id: i64) -> CommandResult<GameDetail> {
    state
        .store
        .lock()
        .expect("store lock poisoned")
        .game(id)
        .map_err(report)?
        .ok_or_else(|| format!("no game with id {id}; import games first"))
}

/// Games not yet reviewed at the default depth under the current criterion,
/// newest first. The webview reviews them one at a time after each sync.
#[tauri::command]
fn unreviewed_games(state: tauri::State<'_, AppState>) -> CommandResult<Vec<i64>> {
    state
        .store
        .lock()
        .expect("store lock poisoned")
        .unreviewed_games(DEFAULT_DEPTH, analysis::CRITERION)
        .map_err(report)
}

/// A stored review, without running the engine. `None` if this game has not
/// been reviewed at this depth under the current mistake criterion.
#[tauri::command]
fn stored_review(
    state: tauri::State<'_, AppState>,
    id: i64,
    depth: Option<u32>,
) -> CommandResult<Option<Review>> {
    let store = state.store.lock().expect("store lock poisoned");
    let Some(game) = store.game(id).map_err(report)? else {
        return Err(format!("no game with id {id}; import games first"));
    };
    store
        .review(
            &game.summary.url,
            depth.unwrap_or(DEFAULT_DEPTH),
            analysis::CRITERION,
        )
        .map(|review| review.map(explain::with_facts))
        .map_err(report)
}

/// Review a game, from storage when it has been run at these settings before.
/// Progress goes out on `on_progress` once per position searched; a stored
/// review sends none.
#[tauri::command]
async fn review_game(
    state: tauri::State<'_, AppState>,
    id: i64,
    depth: Option<u32>,
    on_progress: Channel<Progress>,
) -> CommandResult<Review> {
    let depth = depth.unwrap_or(DEFAULT_DEPTH).clamp(1, 30);
    let run = async {
        let (url, pgn) = {
            let store = state.store.lock().expect("store lock poisoned");
            let game = store
                .game(id)?
                .ok_or_else(|| eyre!("no game with id {id}; import games first"))?;
            if let Some(review) = store.review(&game.summary.url, depth, analysis::CRITERION)? {
                return Ok(explain::with_facts(review));
            }
            (game.summary.url, game.pgn)
        };
        let review = analysis::review_game(&pgn, &state.engine, depth, |done, total| {
            // A closed window drops the channel; the review still finishes
            // and is stored, so the failure to report progress is ignored.
            let _ = on_progress.send(Progress { done, total });
        })
        .await?;
        state
            .store
            .lock()
            .expect("store lock poisoned")
            .save_review(&url, &review)?;
        Ok(explain::with_facts(review))
    };
    run.await.map_err(report)
}

fn now() -> eyre::Result<i64> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("read the clock")?
        .as_secs();
    Ok(i64::try_from(seconds).expect("the clock is before 2^63 seconds"))
}

/// One position for the trainer: the moment before one of your mistakes.
#[derive(Serialize)]
struct Drill {
    game_id: i64,
    opponent: String,
    end_time: i64,
    you: Side,
    /// Expected score before the mistake with best play, 0 to 100, for you.
    /// A tried move is judged against this.
    expected_before: f64,
    mistake: Mistake,
    /// Leitner box; 0 for a position never seen.
    r#box: u32,
    seen: bool,
}

#[derive(Serialize)]
struct PracticeQueue {
    /// Due now: positions whose wait is over first, oldest due first, then
    /// new ones, costliest first.
    due: Vec<Drill>,
    /// Seen positions not yet due again.
    waiting: usize,
    /// Games not reviewed yet, whose mistakes are not in the queue.
    unreviewed: usize,
}

/// Your mistakes from every reviewed game that are worth drilling: those
/// whose refutation wins material or mates. In `trade_neutral` and
/// `positional` mistakes the review cannot say what the better move achieves,
/// so a drill could not explain its answer.
#[tauri::command]
fn practice_queue(state: tauri::State<'_, AppState>) -> CommandResult<PracticeQueue> {
    let run = || -> eyre::Result<PracticeQueue> {
        let now = now()?;
        let store = state.store.lock().expect("store lock poisoned");
        let schedules = store.drill_schedules()?;
        let unreviewed = store
            .unreviewed_games(DEFAULT_DEPTH, analysis::CRITERION)?
            .len();
        let mut overdue = Vec::new();
        let mut fresh = Vec::new();
        let mut waiting = 0;
        for (game, review) in store.reviewed_games(DEFAULT_DEPTH, analysis::CRITERION)? {
            let review = explain::with_facts(review);
            let you = match game.you {
                Some("white") => Side::White,
                Some("black") => Side::Black,
                _ => continue,
            };
            for mistake in review.mistakes {
                if mistake.side != you
                    || !matches!(mistake.cost.kind, CostKind::Material | CostKind::Mate)
                {
                    continue;
                }
                let white_before = review.evaluations[mistake.ply];
                let mine_before = if you == Side::White {
                    white_before
                } else {
                    -white_before
                };
                let schedule = schedules.get(&(game.url.clone(), mistake.ply)).copied();
                let drill = Drill {
                    game_id: game.id,
                    opponent: if you == Side::White {
                        game.black.clone()
                    } else {
                        game.white.clone()
                    },
                    end_time: game.end_time,
                    you,
                    expected_before: analysis::expected_score(mine_before),
                    r#box: schedule.map_or(0, |s| s.r#box),
                    seen: schedule.is_some(),
                    mistake,
                };
                match schedule {
                    Some(s) if s.due_at > now => waiting += 1,
                    Some(s) => overdue.push((s.due_at, drill)),
                    None => fresh.push(drill),
                }
            }
        }
        overdue.sort_by_key(|(due_at, _)| *due_at);
        fresh.sort_by(|a, b| b.mistake.chances_lost.total_cmp(&a.mistake.chances_lost));
        Ok(PracticeQueue {
            due: overdue.into_iter().map(|(_, d)| d).chain(fresh).collect(),
            waiting,
            unreviewed,
        })
    };
    run().map_err(report)
}

#[derive(Serialize)]
struct DrillVerdict {
    judgement: MoveJudgement,
    /// Expected score the tried move gives up against the best move.
    chances_lost: f64,
    solved: bool,
}

/// Judge a move tried in a drill against the best move's expected score.
#[tauri::command]
async fn judge_drill(
    state: tauri::State<'_, AppState>,
    fen: String,
    uci: String,
    expected_before: f64,
) -> CommandResult<DrillVerdict> {
    let run = async {
        let judgement = analysis::judge_move(&state.engine, &fen, &uci, DEFAULT_DEPTH).await?;
        // Clamped at zero: a move can score a little above the "best" one
        // when the engine's view of the position shifts between searches.
        let chances_lost = (expected_before - judgement.expected_after).max(0.0);
        Ok(DrillVerdict {
            solved: chances_lost < practice::SOLVED_POINTS,
            chances_lost: (chances_lost * 10.0).round() / 10.0,
            judgement,
        })
    };
    run.await.map_err(report)
}

/// Record how a drill went and schedule its next appearance. Returns when it
/// is next due, Unix seconds.
#[tauri::command]
fn record_drill(
    state: tauri::State<'_, AppState>,
    game_id: i64,
    ply: usize,
    grade: Grade,
) -> CommandResult<i64> {
    let run = || -> eyre::Result<i64> {
        let now = now()?;
        let store = state.store.lock().expect("store lock poisoned");
        let game = store
            .game(game_id)?
            .ok_or_else(|| eyre!("no game with id {game_id}"))?;
        let current = store
            .drill_schedules()?
            .get(&(game.summary.url.clone(), ply))
            .map_or(0, |s| s.r#box);
        let schedule = practice::next(current, grade, now);
        store.save_drill(&game.summary.url, ply, schedule, now)?;
        Ok(schedule.due_at)
    };
    run().map_err(report)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Fail at startup, with a sentence, rather than on the first
            // review as a spawn error.
            let engine = engine_path()?;
            let db = app
                .path()
                .app_data_dir()
                .context("find the app data directory")?
                .join("chess-coach.sqlite3");
            app.manage(AppState {
                store: Mutex::new(Store::open(&db)?),
                engine,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            account,
            connect_account,
            sync_games,
            list_games,
            get_game,
            stored_review,
            unreviewed_games,
            practice_queue,
            judge_drill,
            record_drill,
            review_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
