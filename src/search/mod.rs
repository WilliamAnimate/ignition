use crate::apps::{App, AppId, AppManager};
use crate::config::Config;
use chrono::{DateTime, Local, TimeDelta, Utc};
use eframe::egui::TextBuffer;
use eyre::Context;
use fuzzy_matcher::skim::{SkimMatcherV2, SkimScoreConfig};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fs::create_dir_all;
use std::ops::Sub;
use std::path::Path;
use tracing::info;

#[derive(Default)]
pub struct SearchResult {
    pub query: String,
    pub entries: Vec<SearchResultEntry>,
}

pub struct SearchResultEntry {
    pub id: AppId,
    pub score: SearchScore,
}

pub struct SearchEngine {
    // Searching
    matcher: SkimMatcherV2,

    // Persistence
    uses: HashMap<AppId, u32>,
    uses_max: u32,
    config: Config<SearchData>,
}

impl SearchEngine {
    pub fn new(dir: &Path) -> eyre::Result<Self> {
        create_dir_all(dir).wrap_err("Failed to create dir")?;
        let mut config = Config::new(dir.join("uses.json"));
        let data: &mut SearchData = config.get_mut().wrap_err("Failed to read config")?;

        let mut uses = HashMap::new();
        for entry in &data.uses {
            *uses.entry(entry.id.clone()).or_default() += 1;
        }

        Ok(Self {
            matcher: SkimMatcherV2::default().score_config(SkimScoreConfig {
                ..SkimScoreConfig::default()
            }),
            uses_max: *uses.values().max().unwrap_or(&1),
            uses,
            config,
        })
    }

    pub fn record_use(&mut self, id: AppId) -> eyre::Result<()> {
        let data = self.config.get_mut().wrap_err("Failed to load config")?;

        // Add new entry
        let now = Local::now().to_utc();
        data.uses.push(UseEntry { id, at: now });

        // Remove old
        let start_len = data.uses.len();
        data.uses.retain(|e| e.at >= now.sub(TimeDelta::days(30)));

        let removed_old = start_len - data.uses.len();
        if removed_old > 0 {
            info!("Purged {removed_old} old entries.");
        }

        // Flush config
        self.config
            .flush_changes()
            .wrap_err("Failed to save config")?;
        Ok(())
    }

    pub fn search(&self, query: String, apps: &AppManager) -> SearchResult {
        let search_query = SearchQuery::from(query);

        let mut results = Vec::new();
        for entry in apps.applications.values() {
            let score = self.score(entry, &search_query);
            results.push(SearchResultEntry {
                id: entry.id.clone(),
                score,
            })
        }

        #[derive(PartialEq)]
        struct SearchOrderKey<'a> {
            score: f32,
            name: &'a str,
        }
        impl Eq for SearchOrderKey<'_> {}
        impl PartialOrd for SearchOrderKey<'_> {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for SearchOrderKey<'_> {
            fn cmp(&self, other: &Self) -> Ordering {
                other
                    .score
                    .total_cmp(&self.score)
                    .then(self.name.cmp(other.name))
            }
        }
        results.sort_unstable_by_key(|v| SearchOrderKey {
            score: v.score.score,
            name: apps.applications.get(&v.id).map(|v| &*v.name).unwrap_or(""),
        });

        SearchResult {
            query: search_query.text,
            entries: results,
        }
    }

    pub fn score(&self, app: &App, query: &SearchQuery) -> SearchScore {
        let mut result = SearchScore {
            score: 0.0,
            indices: Default::default(),
        };

        result.add(50.0, self.score_string(query, &app.name, true));
        result.add(
            0.2,
            self.score_string(query, &app.comment.clone().unwrap_or_default(), false),
        );
        result.add(
            1.0,
            self.score_string(query, &app.generic_name.clone().unwrap_or_default(), false),
        );

        // Go through keywords
        let keywords = app.keywords.clone().unwrap_or_default();
        let split: Vec<&str> = keywords.split(";").collect();
        for &keyword in &split {
            result.add(
                0.5 / split.len() as f32,
                self.score_string(query, keyword, false),
            );
        }

        //
        let length_penalty = app.name.len() as f32 * 0.003 * result.score;
        if result.score > length_penalty {
            result.score -= length_penalty;
        }

        if Self::is_penalized(app) {
            result.score *= 0.9;
        }

        let popularity = self.get_popularity(&app.id);
        result.score *= 1.0 + popularity * 0.5;
        result.score += popularity;

        result
    }

    pub fn get_popularity(&self, id: &AppId) -> f32 {
        let uses = self.uses.get(id).copied().unwrap_or(0);
        uses as f32 / self.uses_max as f32
    }

    fn is_penalized(app: &App) -> bool {
        if app.terminal {
            return true;
        }

        if let Some(categories) = &app.categories {
            return categories.iter().any(|v| v == "Settings");
        }

        false
    }

    fn score_string(&self, query: &SearchQuery, str: &str, with_pos: bool) -> SearchResultPart {
        let mut part = SearchResultPart {
            score: 0.0,
            indices: Default::default(),
        };
        if str.is_empty() {
            return part;
        }

        self.score_full(str, &query.text, 1.0, with_pos, &mut part);
        if with_pos {
            let char_count = query.text.chars().count();
            for i in 0..char_count {
                let mut new_query = query.text.clone();
                new_query.delete_char_range(i..(i + 1));

                self.score_full(
                    str,
                    &new_query,
                    0.75 / char_count as f32,
                    with_pos,
                    &mut part,
                );

                for (i, str) in str.split(" ").enumerate() {
                    let dice = strsim::sorensen_dice(&new_query.to_lowercase(), &str.to_lowercase())
                        as f32;

                    part.score +=
                        (dice.powf(8.0) + 0.05 * dice) * 50.0 / 2.0f32.powf(i as f32 * 2.0);
                    part.score +=
                        strsim::normalized_damerau_levenshtein(&new_query, &str.to_lowercase())
                            as f32
                            * 0.05;
                }
            }
        }

        let sub_strings: Vec<&str> = str.split(&[' ', '-']).collect();
        self.score_small(&sub_strings, &[&query.text], with_pos, &mut part);
        self.score_small(&[str], query.parts.as_slice(), with_pos, &mut part);

        part
    }

    fn score_full(
        &self,
        string: &str,
        query: &str,
        boost: f32,
        with_pos: bool,
        result: &mut SearchResultPart,
    ) {
        if let Some((search, indices)) = self.matcher.fuzzy(string, query, with_pos) {
            result.score += search as f32;
            if with_pos {
                for index in indices {
                    let entry = result.indices.entry(index).or_default();
                    *entry = entry.max(boost);
                }
            }
        }
    }

    fn score_small<V: AsRef<str>>(
        &self,
        strings: &[&str],
        queries: &[V],
        with_pos: bool,
        result: &mut SearchResultPart,
    ) {
        let count = strings.len() * queries.len();
        for string in strings {
            for query in queries {
                if let Some((search, indices)) =
                    self.matcher.fuzzy(string, query.as_ref(), with_pos)
                {
                    result.score += (search as f32 / count as f32) * 0.01;
                    if with_pos {
                        for index in indices {
                            let entry = result.indices.entry(index).or_default();
                            *entry = entry.max(0.5);
                        }
                    }
                }
            }
        }
    }
}

pub struct SearchQuery {
    text: String,
    parts: Vec<String>,
}

impl SearchQuery {
    pub fn from(query: String) -> SearchQuery {
        SearchQuery {
            parts: query.split(' ').map(|v| v.to_string()).collect(),
            text: query,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct SearchData {
    pub uses: Vec<UseEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct UseEntry {
    pub id: AppId,
    pub at: DateTime<Utc>,
}

#[derive(Default)]
pub struct SearchScore {
    pub score: f32,
    pub indices: BTreeMap<usize, f32>,
}
impl SearchScore {
    pub fn add(&mut self, boost: f32, mut part: SearchResultPart) {
        self.score += part.score * boost;
        self.indices.append(&mut part.indices);
    }
}

pub struct SearchResultPart {
    score: f32,
    indices: BTreeMap<usize, f32>,
}
