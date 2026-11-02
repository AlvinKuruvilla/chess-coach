//! SQLite storage for imported games, fetched archives, and finished reviews.
//!
//! Three tables:
//!
//! - `archives` -- which chess.com monthly archives have been fetched. A past
//!   month never changes, so its row is what stops it being fetched again.
//! - `games` -- one row per game, keyed by chess.com's game URL.
//! - `drills` -- trainer progress per mistake position: its Leitner box and
//!   when it is next due (see `practice.rs`). Keyed by game and ply, not by
//!   review, so progress survives a change of mistake criterion for any
//!   position that is still a mistake under the new one.
//! - `settings` -- key/value app settings; today only the chess.com account.
//! - `reviews` -- one row per (game, depth, criterion), where the criterion
//!   names how mistakes are classified (`analysis::CRITERION`). A review is a pure
//!   function of the PGN and those two parameters, because Stockfish at fixed
//!   depth with one thread and an empty hash is bit-exact (checked 2026-08-18,
//!   see SESSION.md). So a stored review is never stale and never recomputed.
//!
//! Games are addressed by the trailing integer of their chess.com URL
//! (`.../game/daily/1000000001` is game `1000000001`), with the URL as the
//! primary key. NOTE: chess.com numbers daily and live games from separate
//! sequences, so two games could share a number in principle. The `UNIQUE`
//! constraint on `games.number` turns that into an import error rather than
//! a silent overwrite.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use eyre::Context;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::analysis::Review;
use crate::chesscom;
use crate::practice::Schedule;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS archives (
    url        TEXT PRIMARY KEY,
    username   TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS games (
    url          TEXT PRIMARY KEY,
    number       INTEGER NOT NULL UNIQUE,
    username     TEXT NOT NULL,
    end_time     INTEGER NOT NULL,
    time_class   TEXT NOT NULL,
    rated        INTEGER NOT NULL,
    rules        TEXT NOT NULL,
    white        TEXT NOT NULL,
    black        TEXT NOT NULL,
    white_rating INTEGER NOT NULL,
    black_rating INTEGER NOT NULL,
    white_result TEXT NOT NULL,
    black_result TEXT NOT NULL,
    pgn          TEXT NOT NULL,
    eco_url      TEXT,
    accuracies   TEXT
);

CREATE INDEX IF NOT EXISTS games_by_end_time ON games (end_time DESC);

CREATE TABLE IF NOT EXISTS drills (
    game_url  TEXT NOT NULL REFERENCES games (url),
    ply       INTEGER NOT NULL,
    box       INTEGER NOT NULL,
    due_at    INTEGER NOT NULL,
    attempts  INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (game_url, ply)
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS reviews (
    game_url  TEXT NOT NULL REFERENCES games (url),
    depth     INTEGER NOT NULL,
    criterion TEXT NOT NULL,
    result    TEXT NOT NULL,
    PRIMARY KEY (game_url, depth, criterion)
);
";

#[derive(Clone, Debug, Serialize)]
pub struct GameSummary {
    pub id: i64,
    pub url: String,
    pub end_time: i64,
    pub time_class: String,
    pub rated: bool,
    pub white: String,
    pub black: String,
    pub white_rating: i64,
    pub black_rating: i64,
    pub white_result: String,
    pub black_result: String,
    pub eco_url: Option<String>,
    pub has_accuracies: bool,
    /// `white` or `black`: the side played by the account the game was
    /// imported for. `None` only if neither player matches.
    pub you: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GameDetail {
    #[serde(flatten)]
    pub summary: GameSummary,
    pub pgn: String,
    pub accuracies: Option<BTreeMap<String, f64>>,
}

pub struct Store {
    db: Connection,
}

impl Store {
    pub fn open(path: &Path) -> eyre::Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create the data directory {}", dir.display()))?;
        }
        let db = Connection::open(path)
            .with_context(|| format!("open the database at {}", path.display()))?;
        Store::init(db)
    }

    #[cfg(test)]
    pub fn in_memory() -> eyre::Result<Store> {
        Store::init(Connection::open_in_memory().context("open an in-memory database")?)
    }

    fn init(db: Connection) -> eyre::Result<Store> {
        db.execute_batch("PRAGMA foreign_keys = ON;")
            .context("enable foreign keys")?;
        // Reviews were first keyed by a centipawn threshold. They are a cache
        // of deterministic engine output, so the old table is dropped rather
        // than migrated; every game is simply reviewed again.
        let old_shape: bool = db
            .query_row(
                "SELECT count(*) > 0 FROM pragma_table_info('reviews') \
                 WHERE name = 'threshold_cp'",
                [],
                |row| row.get(0),
            )
            .context("inspect the reviews table")?;
        if old_shape {
            db.execute_batch("DROP TABLE reviews;")
                .context("drop the threshold-keyed reviews table")?;
        }
        db.execute_batch(SCHEMA).context("create the schema")?;
        Ok(Store { db })
    }

    pub fn setting(&self, key: &str) -> eyre::Result<Option<String>> {
        self.db
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .with_context(|| format!("read the setting {key}"))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> eyre::Result<()> {
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .with_context(|| format!("save the setting {key}"))?;
        Ok(())
    }

    pub fn fetched_archives(&self) -> eyre::Result<HashSet<String>> {
        let mut statement = self.db.prepare("SELECT url FROM archives")?;
        let urls = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()
            .context("read the fetched archives")?;
        Ok(urls)
    }

    pub fn mark_fetched(&self, url: &str, username: &str, now: i64) -> eyre::Result<()> {
        self.db
            .execute(
                "INSERT INTO archives (url, username, fetched_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (url) DO UPDATE SET fetched_at = excluded.fetched_at",
                params![url, username, now],
            )
            .context("record a fetched archive")?;
        Ok(())
    }

    /// Insert games not already stored. Returns how many were new.
    ///
    /// Existing rows are left alone: a game's PGN is final once it has ended.
    /// The one field that changes later is `accuracies`, filled in when Game
    /// Review is run on chess.com after an import, so it is backfilled.
    pub fn upsert_games(
        &mut self,
        username: &str,
        games: &[chesscom::Game],
    ) -> eyre::Result<usize> {
        let tx = self
            .db
            .transaction()
            .context("start an import transaction")?;
        let mut inserted = 0;
        for game in games {
            let Some(pgn) = &game.pgn else { continue };
            let accuracies = game
                .accuracies
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("encode accuracies")?;
            inserted += tx
                .execute(
                    "INSERT INTO games VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16) \
                     ON CONFLICT (url) DO NOTHING",
                    params![
                        game.url,
                        game.number()?,
                        username,
                        game.end_time,
                        game.time_class,
                        game.rated,
                        game.rules,
                        game.white.username,
                        game.black.username,
                        game.white.rating,
                        game.black.rating,
                        game.white.result,
                        game.black.result,
                        pgn,
                        game.eco,
                        accuracies,
                    ],
                )
                .with_context(|| format!("store the game {}", game.url))?;
            if accuracies.is_some() {
                tx.execute(
                    "UPDATE games SET accuracies = ?1 WHERE url = ?2 AND accuracies IS NULL",
                    params![accuracies, game.url],
                )
                .context("backfill accuracies")?;
            }
        }
        tx.commit().context("commit the import")?;
        Ok(inserted)
    }

    pub fn list_games(&self, limit: i64, offset: i64) -> eyre::Result<Vec<GameSummary>> {
        let mut statement = self
            .db
            .prepare("SELECT * FROM games ORDER BY end_time DESC LIMIT ?1 OFFSET ?2")?;
        let games = statement
            .query_map(params![limit, offset], summary)?
            .collect::<Result<_, _>>()
            .context("list games")?;
        Ok(games)
    }

    pub fn count_games(&self) -> eyre::Result<i64> {
        self.db
            .query_row("SELECT count(*) FROM games", [], |row| row.get(0))
            .context("count games")
    }

    pub fn game(&self, number: i64) -> eyre::Result<Option<GameDetail>> {
        self.db
            .query_row("SELECT * FROM games WHERE number = ?1", [number], |row| {
                let accuracies: Option<String> = row.get("accuracies")?;
                Ok(GameDetail {
                    summary: summary(row)?,
                    pgn: row.get("pgn")?,
                    accuracies: accuracies.and_then(|text| serde_json::from_str(&text).ok()),
                })
            })
            .optional()
            .with_context(|| format!("look up game {number}"))
    }

    /// A stored review, if one exists for these parameters and still parses.
    ///
    /// One that no longer parses was stored under an older shape of
    /// [`Review`]. Recomputing it is cheaper than migrating stored JSON, and
    /// the result is the same apart from the changed fields, since the engine
    /// is deterministic -- so it is treated as absent.
    pub fn review(&self, url: &str, depth: u32, criterion: &str) -> eyre::Result<Option<Review>> {
        let text: Option<String> = self
            .db
            .query_row(
                "SELECT result FROM reviews \
                 WHERE game_url = ?1 AND depth = ?2 AND criterion = ?3",
                params![url, depth, criterion],
                |row| row.get(0),
            )
            .optional()
            .context("look up a stored review")?;
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
    }

    /// Ids of games with no review at this depth under `criterion`, newest
    /// first, so the games most likely to be opened next are reviewed first.
    pub fn unreviewed_games(&self, depth: u32, criterion: &str) -> eyre::Result<Vec<i64>> {
        let mut statement = self.db.prepare(
            "SELECT number FROM games g WHERE NOT EXISTS ( \
                 SELECT 1 FROM reviews r \
                 WHERE r.game_url = g.url AND r.depth = ?1 AND r.criterion = ?2) \
             ORDER BY end_time DESC",
        )?;
        let ids = statement
            .query_map(params![depth, criterion], |row| row.get(0))?
            .collect::<Result<_, _>>()
            .context("list unreviewed games")?;
        Ok(ids)
    }

    /// Every game with a stored review at this depth under `criterion`,
    /// paired with the review. Reviews that no longer parse are skipped, as
    /// in [`Store::review`].
    pub fn reviewed_games(
        &self,
        depth: u32,
        criterion: &str,
    ) -> eyre::Result<Vec<(GameSummary, Review)>> {
        let mut statement = self.db.prepare(
            "SELECT g.*, r.result AS review FROM games g \
             JOIN reviews r ON r.game_url = g.url \
             WHERE r.depth = ?1 AND r.criterion = ?2 \
             ORDER BY g.end_time DESC",
        )?;
        let rows: Vec<(GameSummary, String)> = statement
            .query_map(params![depth, criterion], |row| {
                Ok((summary(row)?, row.get("review")?))
            })?
            .collect::<Result<_, _>>()
            .context("list reviewed games")?;
        Ok(rows
            .into_iter()
            .filter_map(|(game, text)| Some((game, serde_json::from_str(&text).ok()?)))
            .collect())
    }

    /// Trainer progress for every position seen so far, keyed by game URL
    /// and ply.
    pub fn drill_schedules(&self) -> eyre::Result<HashMap<(String, usize), Schedule>> {
        let mut statement = self
            .db
            .prepare("SELECT game_url, ply, box, due_at FROM drills")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        usize::try_from(row.get::<_, u32>(1)?).expect("a ply fits in usize"),
                    ),
                    Schedule {
                        r#box: row.get(2)?,
                        due_at: row.get(3)?,
                    },
                ))
            })?
            .collect::<Result<_, _>>()
            .context("read trainer progress")?;
        Ok(rows)
    }

    pub fn save_drill(
        &self,
        url: &str,
        ply: usize,
        schedule: Schedule,
        now: i64,
    ) -> eyre::Result<()> {
        self.db
            .execute(
                "INSERT INTO drills (game_url, ply, box, due_at, attempts, last_seen) \
                 VALUES (?1, ?2, ?3, ?4, 1, ?5) \
                 ON CONFLICT (game_url, ply) DO UPDATE SET box = excluded.box, \
                 due_at = excluded.due_at, attempts = attempts + 1, \
                 last_seen = excluded.last_seen",
                params![
                    url,
                    u32::try_from(ply).context("store the ply")?,
                    schedule.r#box,
                    schedule.due_at,
                    now
                ],
            )
            .context("save trainer progress")?;
        Ok(())
    }

    pub fn save_review(&self, url: &str, review: &Review) -> eyre::Result<()> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO reviews VALUES (?1, ?2, ?3, ?4)",
                params![
                    url,
                    review.depth,
                    review.criterion,
                    serde_json::to_string(review).context("encode the review")?
                ],
            )
            .context("store the review")?;
        Ok(())
    }
}

fn summary(row: &Row<'_>) -> rusqlite::Result<GameSummary> {
    let username: String = row.get("username")?;
    let white: String = row.get("white")?;
    let black: String = row.get("black")?;
    // chess.com usernames are case-insensitive, and the import name and the
    // game record disagree on case: `alvinkuruvilla02` vs `AlvinKuruvilla02`.
    let you = if white.eq_ignore_ascii_case(&username) {
        Some("white")
    } else if black.eq_ignore_ascii_case(&username) {
        Some("black")
    } else {
        None
    };
    let accuracies: Option<String> = row.get("accuracies")?;
    Ok(GameSummary {
        id: row.get("number")?,
        url: row.get("url")?,
        end_time: row.get("end_time")?,
        time_class: row.get("time_class")?,
        rated: row.get("rated")?,
        white,
        black,
        white_rating: row.get("white_rating")?,
        black_rating: row.get("black_rating")?,
        white_result: row.get("white_result")?,
        black_result: row.get("black_result")?,
        eco_url: row.get("eco_url")?,
        has_accuracies: accuracies.is_some(),
        you,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_stored_once_listed_newest_first_and_sided() {
        let mut store = Store::in_memory().expect("store opens");
        let games = chesscom::fixture();
        assert_eq!(
            store
                .upsert_games("alvinkuruvilla02", &games)
                .expect("insert"),
            2
        );
        assert_eq!(
            store
                .upsert_games("alvinkuruvilla02", &games)
                .expect("insert"),
            0
        );

        let listed = store.list_games(50, 0).expect("list");
        assert_eq!(listed.len(), 2);
        assert!(listed[0].end_time >= listed[1].end_time);

        let game = store.game(1000000001).expect("query").expect("present");
        assert_eq!(game.summary.you, Some("white"));
        assert_eq!(game.accuracies.as_ref().map(|a| a["black"]), Some(89.31));
        assert!(store.game(1).expect("query").is_none());
    }

    #[test]
    fn settings_are_absent_until_set_and_then_overwritten() {
        let store = Store::in_memory().expect("store opens");
        assert_eq!(store.setting("chesscom_username").expect("read"), None);
        store
            .set_setting("chesscom_username", "first")
            .expect("write");
        store
            .set_setting("chesscom_username", "second")
            .expect("write");
        assert_eq!(
            store.setting("chesscom_username").expect("read").as_deref(),
            Some("second")
        );
    }

    #[test]
    fn the_threshold_keyed_reviews_table_is_replaced_and_games_are_kept() {
        // The layout databases had before reviews were keyed by criterion:
        // today's schema with the old reviews key, and one game in it.
        let old = SCHEMA
            .replace("criterion TEXT NOT NULL", "threshold_cp INTEGER NOT NULL")
            .replace(
                "(game_url, depth, criterion)",
                "(game_url, depth, threshold_cp)",
            );
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch(&old).expect("old schema");
        db.execute_batch(
            "INSERT INTO games VALUES ('https://www.chess.com/game/daily/1', 1, 'me', 0,
                 'daily', 1, 'chess', 'me', 'them', 300, 300, 'win', 'resigned',
                 '1. e4 *', NULL, NULL);
             INSERT INTO reviews VALUES ('https://www.chess.com/game/daily/1', 16, 200, '{}');",
        )
        .expect("old rows");
        let store = Store::init(db).expect("init over the old schema");
        let columns: Vec<String> = store
            .db
            .prepare("SELECT name FROM pragma_table_info('reviews')")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("read");
        assert!(columns.contains(&"criterion".to_owned()));
        assert!(!columns.contains(&"threshold_cp".to_owned()));
        let games: i64 = store
            .db
            .query_row("SELECT count(*) FROM games", [], |row| row.get(0))
            .expect("count");
        assert_eq!(games, 1);
    }

    #[test]
    fn only_games_without_a_current_review_are_unreviewed() {
        let mut store = Store::in_memory().expect("store opens");
        store
            .upsert_games("alvinkuruvilla02", &chesscom::fixture())
            .expect("insert");
        assert_eq!(store.unreviewed_games(16, "c").expect("query").len(), 2);
        let url = "https://www.chess.com/game/daily/1000000001";
        store
            .db
            .execute("INSERT INTO reviews VALUES (?1, 16, 'c', '{}')", [url])
            .expect("insert a review");
        assert_eq!(
            store.unreviewed_games(16, "c").expect("query"),
            [1000000002]
        );
        // A review under another criterion or depth does not count.
        assert_eq!(store.unreviewed_games(16, "other").expect("query").len(), 2);
        assert_eq!(store.unreviewed_games(12, "c").expect("query").len(), 2);
    }

    #[test]
    fn trainer_progress_is_saved_and_updated_in_place() {
        let mut store = Store::in_memory().expect("store opens");
        store
            .upsert_games("alvinkuruvilla02", &chesscom::fixture())
            .expect("insert");
        let url = "https://www.chess.com/game/daily/1000000001";
        let first = Schedule {
            r#box: 1,
            due_at: 100,
        };
        store.save_drill(url, 8, first, 10).expect("save");
        let second = Schedule {
            r#box: 2,
            due_at: 200,
        };
        store.save_drill(url, 8, second, 20).expect("save again");
        let schedules = store.drill_schedules().expect("read");
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[&(url.to_owned(), 8)], second);
    }

    #[test]
    fn a_review_stored_under_an_older_shape_reads_as_absent() {
        let mut store = Store::in_memory().expect("store opens");
        store
            .upsert_games("alvinkuruvilla02", &chesscom::fixture())
            .expect("insert");
        let url = "https://www.chess.com/game/daily/1000000001";
        store
            .db
            .execute(
                "INSERT INTO reviews VALUES (?1, 16, 'c', '{\"mistakes\": []}')",
                [url],
            )
            .expect("insert an old-shape review");
        assert!(store.review(url, 16, "c").expect("query").is_none());
    }
}
