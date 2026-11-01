//! Reading a player's finished games from chess.com's public API.
//!
//! The API is unauthenticated and organised by month: `games/archives` lists
//! one URL per month the player has games in, and each of those returns
//! `{"games": [...]}` with the full PGN of every game that ended that month.
//! Verified against the live API on 2026-11-01.
//!
//! A month's archive is immutable once the month is over -- games are filed by
//! the month they *ended*, so nothing can be added to a past month. Only the
//! newest archive can still change, which is what lets the importer fetch
//! every other month exactly once.

use std::collections::{BTreeMap, HashSet};

use eyre::{Context, bail, eyre};
use serde::Deserialize;

const API: &str = "https://api.chess.com/pub";

/// chess.com asks API clients to identify themselves in the User-Agent.
/// Contact details are left out on purpose: they would go to a third party
/// on every request.
const USER_AGENT: &str = "chess-coach/0.1 (personal game review)";

/// One finished game, reduced to what review and listing need.
#[derive(Clone, Debug, Deserialize)]
pub struct Game {
    /// `https://www.chess.com/game/<class>/<number>`. Stable and unique.
    pub url: String,
    /// Unix seconds.
    pub end_time: i64,
    pub time_class: String,
    pub rated: bool,
    /// `chess` for standard games; variants (chess960, bughouse, ...) differ.
    pub rules: String,
    pub white: Player,
    pub black: Player,
    /// Absent entries are skipped by [`archive_games`]; there is nothing to
    /// review without one.
    pub pgn: Option<String>,
    /// A URL naming the opening, e.g. `.../openings/Queens-Pawn-Opening-...`.
    pub eco: Option<String>,
    /// Present only on games someone has run chess.com Game Review on.
    pub accuracies: Option<BTreeMap<String, f64>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Player {
    pub username: String,
    pub rating: i64,
    /// chess.com's per-side outcome: `win`, `checkmated`, `resigned`, ...
    pub result: String,
}

impl Game {
    /// The trailing integer of the URL, which the app uses as the game id.
    pub fn number(&self) -> eyre::Result<i64> {
        self.url
            .rsplit('/')
            .next()
            .and_then(|tail| tail.parse().ok())
            .ok_or_else(|| eyre!("game URL {} does not end in a number", self.url))
    }
}

#[derive(Deserialize)]
struct Archives {
    archives: Vec<String>,
}

#[derive(Deserialize)]
struct Month {
    games: Vec<Game>,
}

pub fn client() -> eyre::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build the HTTP client")
}

/// Monthly archive URLs for `username`, oldest first.
///
/// The API returns them in chronological order; [`archives_to_fetch`] relies
/// on the last one being the only month that can still change.
pub async fn archive_urls(http: &reqwest::Client, username: &str) -> eyre::Result<Vec<String>> {
    let url = format!("{API}/player/{}/games/archives", username.to_lowercase());
    let response = http.get(&url).send().await.context("reach chess.com")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("chess.com has no player named {username:?}");
    }
    let archives: Archives = response
        .error_for_status()
        .context("list the player's monthly archives")?
        .json()
        .await
        .context("read the archive list")?;
    Ok(archives.archives)
}

/// Every game in one monthly archive that carries a PGN.
pub async fn archive_games(http: &reqwest::Client, url: &str) -> eyre::Result<Vec<Game>> {
    let month: Month = http
        .get(url)
        .send()
        .await
        .context("reach chess.com")?
        .error_for_status()
        .with_context(|| format!("fetch the archive {url}"))?
        .json()
        .await
        .with_context(|| format!("read the archive {url}"))?;
    Ok(parse_month(month))
}

fn parse_month(month: Month) -> Vec<Game> {
    month
        .games
        .into_iter()
        .filter(|g| g.pgn.is_some())
        .collect()
}

/// Which archives an import must fetch: every month not fetched before, plus
/// the newest month, which keeps gaining games until it ends.
pub fn archives_to_fetch(urls: &[String], fetched: &HashSet<String>) -> Vec<String> {
    let newest = urls.last();
    urls.iter()
        .filter(|url| !fetched.contains(*url) || Some(*url) == newest)
        .cloned()
        .collect()
}

#[cfg(test)]
pub(crate) fn fixture() -> Vec<Game> {
    // Two real games from the account's 2026-10 archive, fetched 2026-11-01:
    // one with chess.com accuracies and one without.
    let month: Month =
        serde_json::from_str(include_str!("../tests/fixtures/chesscom-2026-10.json"))
            .expect("the fixture is a chess.com month");
    parse_month(month)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_games_parse_and_are_numbered_by_url() {
        let games = fixture();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].number().expect("numbered"), 1000000001);
        assert_eq!(games[0].white.username, "AlvinKuruvilla02");
        assert_eq!(
            games[0].accuracies.as_ref().map(|a| a["white"]),
            Some(71.67)
        );
        assert!(games[1].accuracies.is_none());
    }

    #[test]
    fn past_months_are_fetched_once_and_the_newest_every_time() {
        let urls = vec!["sep".to_owned(), "oct".to_owned()];
        let none = HashSet::new();
        assert_eq!(archives_to_fetch(&urls, &none), ["sep", "oct"]);

        let both: HashSet<String> = urls.iter().cloned().collect();
        assert_eq!(archives_to_fetch(&urls, &both), ["oct"]);
    }
}
