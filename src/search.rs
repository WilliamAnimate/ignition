use crate::config::Config;
use crate::shortcuts::{Shortcut, ShortcutId};
use chrono::{DateTime, Local, TimeDelta, Utc};
use eframe::egui::TextBuffer;
use fuzzy_matcher::skim::{SkimMatcherV2, SkimScoreConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::create_dir_all;
use std::ops::{Sub};
use std::path::{ PathBuf};
use tracing::info;

#[derive(Serialize, Deserialize, Default)]
pub struct SearchData {
    pub uses: Vec<UseEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct UseEntry {
    pub id: ShortcutId,
    pub at: DateTime<Utc>,
}

pub struct SearchEngine {
    matcher: SkimMatcherV2,

    uses: HashMap<ShortcutId, u32>,
    uses_max: u32,
    config: Config<SearchData>,
}

impl SearchEngine {
    pub fn new(dir: PathBuf) -> Self {
        create_dir_all(&dir).unwrap();
        let mut config = Config::new(dir.join("uses.json"));
        let data: &mut SearchData = config.get_mut().unwrap();

        let mut uses = HashMap::new();
        for entry in &data.uses {
            *uses.entry(entry.id.clone()).or_default() += 1;
        }

        Self {
            matcher: SkimMatcherV2::default().score_config(SkimScoreConfig {
                ..SkimScoreConfig::default()
            }),
            uses_max: *uses.values().max().unwrap_or(&1),
            uses,
            config,
        }
    }

    pub fn record_use(&mut self, id: ShortcutId) {
        let data = self.config.get_mut().unwrap();

        // Add new entry
        let now = Local::now().to_utc();
        data.uses.push(UseEntry { id, at: now });

        // Remove old
        let removed_old = data
            .uses
            .extract_if(|e| e.at < now.sub(TimeDelta::days(30)))
            .count();
        if removed_old > 0 {
            info!("Purged {removed_old} old entries.")
        }

        // Flush config
        self.config.flush_changes();
    }

    pub fn get_popularity(&self, id: &ShortcutId) -> f32 {
        let uses = self.uses.get(id).copied().unwrap_or(0);
        let popularity = uses as f32 / self.uses_max as f32;
        popularity
    }

    pub fn score(&self, model: &Shortcut, query: &SearchQuery) -> SearchScore {
        let mut result = SearchScore {
            score: 0.0,
            indices: Default::default(),
        };

        result.add(50.0, self.score_string(query, &model.name, true));
        result.add(
            0.2,
            self.score_string(query, &model.comment.clone().unwrap_or_default(), false),
        );
        result.add(
            1.0,
            self.score_string(
                query,
                &model.generic_name.clone().unwrap_or_default(),
                false,
            ),
        );

        // Go through keywords
        let keywords = model.keywords.clone().unwrap_or_default();
        let split: Vec<&str> = keywords.split(";").collect();
        for &keyword in &split {
            result.add(
                0.5 / split.len() as f32,
                self.score_string(query, keyword, false),
            );
        }

        //
        let length_penalty = model.name.len() as f32 * 0.003 * result.score;
        if result.score > length_penalty {
            result.score -= length_penalty;
        }

        if model.penalized {
            result.score *= 0.9;
        }

        
        let popularity = self.get_popularity(&model.id);
        result.score *= 1.0 + popularity * 0.5;
        result.score += popularity;
        
        result
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
