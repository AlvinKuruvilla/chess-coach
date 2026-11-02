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
//! ```
//!
//! Errors cross the IPC boundary as strings with the full `eyre` context
//! chain, because the only consumer is a person reading them in the window.

mod analysis;
mod chesscom;
mod store;
mod uci;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Context, eyre};
use serde::Serialize;
use tauri::Manager;
use tauri::ipc::Channel;

use crate::analysis::Review;
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
                return Ok(review);
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
        Ok(review)
    };
    run.await.map_err(report)
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
            review_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
