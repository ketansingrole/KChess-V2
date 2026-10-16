//! What an opening page shows beside the explorer: the line's Wikibooks description (“Chess
//! Opening Theory”, CC BY-SA 4.0, fetched when asked and never bundled) and its popularity over
//! time from the Lichess explorer's history. Both are saved like the other lookups, so a page seen
//! once still shows them offline.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

use super::PositionLookups;
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::training::openings::{line_fens, wikibooks_title};

/// Saved descriptions (and pages that do not exist) are fresh for a week; so is popularity.
const FRESH_MS: i64 = 7 * 86_400_000;
const MAX_BYTES: usize = 400_000;
/// Months with fewer games at the earlier position give no honest share, so they are left out.
const MIN_MONTH_GAMES: i64 = 100;
const MAX_BLOCKS: usize = 40;
const MAX_BLOCK_UNITS: usize = 3_000;
/// Sections after which a Wikibooks opening page holds no more description.
const STOP_HEADINGS: &[&str] = &[
    "References",
    "Theory table",
    "Statistics",
    "External links",
    "See also",
    "Further reading",
    "Sources",
    "Notes",
    "Footnotes",
];
/// Wikibooks' editing notes, which are not about the opening.
const BOILERPLATE: &[&str] = &["When contributing to this Wikibook", "This page is a stub"];

/// One paragraph or section heading of a description (`DescriptionBlock`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionBlock {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
    pub text: String,
}

/// A line's Wikibooks page as plain text (`OpeningDescription`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpeningDescription {
    pub title: String,
    pub url: String,
    pub blocks: Vec<DescriptionBlock>,
    pub fetched_at: i64,
}

/// One month of a line's popularity (`PopularityMonth`): of the games at the position before its
/// last move, the share that continued with that move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PopularityMonth {
    pub month: String,
    pub share: f64,
    pub games: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedDescription {
    fetched_at: i64,
    page: Option<OpeningDescription>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedPopularity {
    fetched_at: i64,
    months: Vec<PopularityMonth>,
}

/// One position's explorer history, saved apart from the lines it is part of: a line's parent is
/// its sibling lines' parent too, and the child of the line before it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedHistory {
    fetched_at: i64,
    history: Value,
}

/// The UCI moves of a line, as the methods receive them.
fn line_of(raw: &Value) -> CoreResult<Vec<String>> {
    let moves: Vec<String> = raw
        .as_array()
        .ok_or_else(|| CoreError::new("moves must be a list."))?
        .iter()
        .map(|entry| entry.as_str().map(str::to_string))
        .collect::<Option<_>>()
        .ok_or_else(|| CoreError::new("moves must be UCI moves."))?;
    if moves.len() > 200 || line_fens(&moves).is_none() {
        return Err(CoreError::new("That line is not legal."));
    }
    Ok(moves)
}

/// The blocks of a Wikibooks plain-text extract: paragraphs, and the section headings between
/// them (a short line after a blank gap), until the reference sections.
pub fn description_blocks(extract: &str) -> Vec<DescriptionBlock> {
    let mut blocks = Vec::new();
    for (index, section) in extract.split("\n\n\n").enumerate() {
        let mut lines = section
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let Some(first) = lines.clone().next() else {
            continue;
        };
        // The page's own title line (“2. f4 · King's gambit”) repeats the opening page's title.
        let heading = !first.ends_with(['.', '!', '?', ':', ')']) && first.chars().count() <= 80;
        if index == 0 && first.contains(" · ") {
            lines.next();
        } else if index > 0 && heading {
            if STOP_HEADINGS
                .iter()
                .any(|stop| first.eq_ignore_ascii_case(stop))
            {
                break;
            }
            lines.next();
            blocks.push(DescriptionBlock {
                heading: true,
                text: first.to_string(),
            });
        }
        for line in lines {
            if BOILERPLATE.iter().any(|note| line.starts_with(note)) {
                continue;
            }
            blocks.push(DescriptionBlock {
                heading: false,
                text: line.chars().take(MAX_BLOCK_UNITS).collect(),
            });
        }
        if blocks.len() >= MAX_BLOCKS {
            blocks.truncate(MAX_BLOCKS);
            break;
        }
    }
    // A heading with nothing below it says nothing.
    while blocks.last().is_some_and(|block| block.heading) {
        blocks.pop();
    }
    blocks
}

/// Monthly shares of a line's last move: its games over the games at the position before it.
/// Months with fewer than `MIN_MONTH_GAMES` games at the earlier position are left out.
pub fn popularity(parent: &Value, child: &Value) -> Vec<PopularityMonth> {
    let months = |raw: &Value| -> Vec<(String, i64)> {
        raw.get("history")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let month = entry.get("month")?.as_str()?;
                        let count = |key: &str| entry.get(key).and_then(Value::as_i64).unwrap_or(0);
                        let valid = month.len() == 7 && month.as_bytes()[4] == b'-';
                        valid.then(|| {
                            (
                                month.to_string(),
                                count("white") + count("draws") + count("black"),
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let child: std::collections::HashMap<String, i64> = months(child).into_iter().collect();
    months(parent)
        .into_iter()
        .filter(|(_, total)| *total >= MIN_MONTH_GAMES)
        .map(|(month, total)| {
            let games = child.get(&month).copied().unwrap_or(0).clamp(0, total);
            PopularityMonth {
                share: games as f64 / total as f64,
                games,
                month,
            }
        })
        .collect()
}

impl PositionLookups {
    /// `openingDescription(moves)`: the line's Wikibooks page, or None when it has none.
    pub async fn opening_description(&self, raw: &Value) -> CoreResult<Option<OpeningDescription>> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.opening_description_inner(raw) => result,
        }
    }

    async fn opening_description_inner(
        &self,
        raw: &Value,
    ) -> CoreResult<Option<OpeningDescription>> {
        let moves = line_of(raw)?;
        let Some(title) = wikibooks_title(&moves) else {
            return Ok(None);
        };
        let key = format!("wikibooks:{title}");
        let saved: Option<SavedDescription> = self
            .cache
            .read(&key)
            .await?
            .and_then(|value| serde_json::from_value(value).ok());
        let now = (self.now)();
        if let Some(saved) = &saved
            && now - saved.fetched_at < FRESH_MS
        {
            return Ok(saved.page.clone());
        }
        match self.fetch_description(&title, now).await {
            Ok(page) => {
                let value = json!(SavedDescription {
                    fetched_at: now,
                    page: page.clone(),
                });
                self.cache.write(&key, &value).await?;
                Ok(page)
            }
            // An older copy beats nothing; with none, the page simply shows no description.
            Err(cause) => match saved {
                Some(saved) => Ok(saved.page),
                None => Err(cause),
            },
        }
    }

    async fn fetch_description(
        &self,
        title: &str,
        now: i64,
    ) -> CoreResult<Option<OpeningDescription>> {
        let mut url = reqwest::Url::parse(&self.endpoints.wikibooks)
            .map_err(|cause| CoreError::new(format!("Invalid Wikibooks URL: {cause}")))?;
        url.query_pairs_mut()
            .append_pair("action", "query")
            .append_pair("prop", "extracts")
            .append_pair("explaintext", "1")
            .append_pair("exsectionformat", "plain")
            .append_pair("redirects", "1")
            .append_pair("format", "json")
            .append_pair("formatversion", "2")
            .append_pair("titles", title);
        let text = self
            .bounded_get(self.http.get(url), "The opening description")
            .await?;
        let raw: Value = serde_json::from_str(&text)
            .map_err(|_| CoreError::new("The opening description could not be read."))?;
        let Some(page) = raw.pointer("/query/pages/0") else {
            return Ok(None);
        };
        if page.get("missing").is_some() {
            return Ok(None);
        }
        let extract = page.get("extract").and_then(Value::as_str).unwrap_or("");
        let blocks = description_blocks(extract);
        if blocks.iter().all(|block| block.heading) {
            return Ok(None);
        }
        let shown = page.get("title").and_then(Value::as_str).unwrap_or(title);
        let path = shown.replace(' ', "_");
        Ok(Some(OpeningDescription {
            title: shown.to_string(),
            url: format!("https://en.wikibooks.org/wiki/{path}"),
            blocks,
            fetched_at: now,
        }))
    }

    /// `openingPopularity(moves)`: the monthly share of the Lichess games (blitz, rapid and
    /// classical) at the position before the line's last move that went on with it.
    pub async fn opening_popularity(&self, raw: &Value) -> CoreResult<Vec<PopularityMonth>> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.opening_popularity_inner(raw) => result,
        }
    }

    async fn opening_popularity_inner(&self, raw: &Value) -> CoreResult<Vec<PopularityMonth>> {
        let moves = line_of(raw)?;
        let fens = line_fens(&moves).unwrap_or_default();
        let (Some(parent), Some(child)) = (fens.iter().rev().nth(1), fens.last()) else {
            return Ok(Vec::new());
        };
        let key = format!("popularity:{parent}:{child}");
        let saved: Option<SavedPopularity> = self
            .cache
            .read(&key)
            .await?
            .and_then(|value| serde_json::from_value(value).ok());
        let now = (self.now)();
        if let Some(saved) = &saved
            && now - saved.fetched_at < FRESH_MS
        {
            return Ok(saved.months.clone());
        }
        let attempt = async {
            let parent = self.history(parent, now).await?;
            let child = self.history(child, now).await?;
            Ok::<_, CoreError>(popularity(&parent, &child))
        };
        match attempt.await {
            Ok(months) => {
                let value = json!(SavedPopularity {
                    fetched_at: now,
                    months: months.clone(),
                });
                self.cache.write(&key, &value).await?;
                Ok(months)
            }
            Err(cause) => match saved {
                Some(saved) => Ok(saved.months),
                None => Err(cause),
            },
        }
    }

    /// A position's history, the saved one while it is fresh (or when asking again fails).
    async fn history(&self, fen: &str, now: i64) -> CoreResult<Value> {
        let key = format!("history:{fen}");
        let saved: Option<SavedHistory> = self
            .cache
            .read(&key)
            .await?
            .and_then(|value| serde_json::from_value(value).ok());
        if let Some(saved) = &saved
            && now - saved.fetched_at < FRESH_MS
        {
            return Ok(json!({ "history": saved.history }));
        }
        match self.fetch_history(fen).await {
            Ok(raw) => {
                let history = raw.get("history").cloned().unwrap_or_else(|| json!([]));
                let value = json!(SavedHistory {
                    fetched_at: now,
                    history,
                });
                self.cache.write(&key, &value).await?;
                Ok(raw)
            }
            Err(cause) => match saved {
                Some(saved) => Ok(json!({ "history": saved.history })),
                None => Err(cause),
            },
        }
    }

    async fn fetch_history(&self, fen: &str) -> CoreResult<Value> {
        let mut url = reqwest::Url::parse(&format!("{}/history", self.endpoints.opening))
            .map_err(|cause| CoreError::new(format!("Invalid explorer URL: {cause}")))?;
        url.query_pairs_mut()
            .append_pair("variant", "standard")
            .append_pair("fen", fen)
            .append_pair("speeds", "blitz,rapid,classical");
        let request = self.explorer_request(url).await?;
        let text = self.bounded_get(request, "Opening popularity").await?;
        serde_json::from_str(&text)
            .map_err(|_| CoreError::new("Opening popularity could not be read."))
    }

    /// A GET answered within 15 seconds and `MAX_BYTES`; `what` names it in errors.
    async fn bounded_get(
        &self,
        request: reqwest::RequestBuilder,
        what: &str,
    ) -> CoreResult<String> {
        let request = request
            .build()
            .map_err(|_| CoreError::new(format!("{what} could not be loaded.")))?;
        let url = request.url().clone();
        self.paced(&url)?;
        let attempt = async {
            let mut response = self.http.execute(request).await.map_err(|_| {
                CoreError::new(format!(
                    "{what} could not be loaded. Check your connection."
                ))
            })?;
            self.note_rate_limit(&url, &response)?;
            let status = response.status().as_u16();
            if status == 401 || status == 403 {
                return Err(CoreError::new(
                    "Connect a Lichess account in Settings to use the opening explorer.",
                ));
            }
            if !response.status().is_success() {
                return Err(CoreError::new(format!(
                    "{what} returned {status}. Try again shortly."
                )));
            }
            let mut bytes: Vec<u8> = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                CoreError::new(format!(
                    "{what} could not be loaded. Check your connection."
                ))
            })? {
                if bytes.len() + chunk.len() > MAX_BYTES {
                    return Err(CoreError::new(format!("{what} returned too much data.")));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        tokio::time::timeout(Duration::from_secs(15), attempt)
            .await
            .map_err(|_| CoreError::new("The operation was aborted due to timeout"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_wikibooks_extract_as_paragraphs_and_headings() {
        let extract = "\n2. f4 · King's gambit\nWhite attacks the pawn.\nIt is aggressive.\n\n\nAccept the gambit\n2...exf4 is the main move.\n\n\nWhen contributing to this Wikibook, please follow the Conventions.\n\n\nReferences\nSome book.";
        let blocks = description_blocks(extract);
        let texts: Vec<(&str, bool)> = blocks
            .iter()
            .map(|b| (b.text.as_str(), b.heading))
            .collect();
        assert_eq!(
            texts,
            [
                ("White attacks the pawn.", false),
                ("It is aggressive.", false),
                ("Accept the gambit", true),
                ("2...exf4 is the main move.", false),
            ]
        );
        assert!(description_blocks("").is_empty());
        assert!(description_blocks("\n\n\nReferences\nA book.").is_empty());
    }

    #[test]
    fn popularity_is_the_share_of_the_earlier_positions_games() {
        let parent = json!({ "history": [
            { "month": "2024-01", "white": 50, "draws": 10, "black": 40 },
            { "month": "2024-02", "white": 0, "draws": 0, "black": 0 },
            { "month": "2024-03", "white": 100, "draws": 0, "black": 100 },
            { "month": "bad", "white": 1 }
        ] });
        let child = json!({ "history": [
            { "month": "2024-01", "white": 10, "draws": 5, "black": 10 },
            { "month": "2024-03", "white": 30, "draws": 0, "black": 20 }
        ] });
        let months = popularity(&parent, &child);
        assert_eq!(
            months,
            [
                PopularityMonth {
                    month: "2024-01".into(),
                    share: 0.25,
                    games: 25
                },
                PopularityMonth {
                    month: "2024-03".into(),
                    share: 0.25,
                    games: 50
                },
            ]
        );
        assert!(popularity(&json!({}), &child).is_empty());
    }
}
