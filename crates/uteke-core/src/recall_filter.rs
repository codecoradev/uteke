//! Result filters of a recall: `strict` on similarity (#1650).
//!
//! The filter is applied to the ranked candidates BEFORE the result is cut to the
//! caller's limit, so a filtered recall still returns up to `limit` rows when
//! that many qualify. The strategies cut to the limit themselves, so
//! [`ResultFilter::fetch`] asks for a larger window and widens it until enough
//! rows survive or the candidates run out.
//!
//! Neither filter changes `score` or the ranking.

use crate::error::Error;
use crate::memory::types::{SearchResult, UnifiedSearchResult};

/// Largest candidate window [`ResultFilter::fetch`] asks a strategy for.
const MAX_WINDOW: usize = 1000;

/// The filters a recall request carries next to its ranking parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResultFilter {
    /// `strict`: keep a hit only when its `similarity` (the vector-leg cosine)
    /// is at least this. A hit without a similarity is dropped.
    pub min_similarity: Option<f32>,
}

impl ResultFilter {
    /// No filter is active: results pass through untouched.
    pub fn is_noop(&self) -> bool {
        self.min_similarity.is_none()
    }

    fn keep_similarity(&self, similarity: Option<f32>) -> bool {
        match self.min_similarity {
            None => true,
            Some(floor) => similarity.is_some_and(|s| s >= floor),
        }
    }

    /// Whether a memory result passes the filter.
    pub fn keep_memory(&self, r: &SearchResult) -> bool {
        self.keep_similarity(r.similarity)
    }

    /// Whether a unified result passes the filter. A document has no
    /// similarity, so an active filter drops it.
    pub fn keep_unified(&self, r: &UnifiedSearchResult) -> bool {
        self.keep_similarity(r.similarity)
    }

    /// Run `fetch(window)` and keep the rows `keep` accepts, up to `limit`.
    ///
    /// With no active filter this is exactly `fetch(limit)`. Otherwise the
    /// window starts at three times the limit and grows fourfold while the
    /// strategy keeps returning a full window but too few rows pass, up to
    /// [`MAX_WINDOW`] candidates.
    pub fn fetch<T>(
        &self,
        limit: usize,
        mut fetch: impl FnMut(usize) -> Result<Vec<T>, Error>,
        keep: impl Fn(&Self, &T) -> bool,
    ) -> Result<Vec<T>, Error> {
        if self.is_noop() {
            return fetch(limit);
        }
        let mut window = limit.saturating_mul(3).max(limit + 10).min(MAX_WINDOW);
        loop {
            let raw = fetch(window)?;
            let exhausted = raw.len() < window;
            let mut kept: Vec<T> = raw.into_iter().filter(|r| keep(self, r)).collect();
            if kept.len() >= limit || exhausted || window >= MAX_WINDOW {
                kept.truncate(limit);
                return Ok(kept);
            }
            window = window.saturating_mul(4).min(MAX_WINDOW);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(sim: Option<f32>) -> ResultFilter {
        ResultFilter {
            min_similarity: sim,
        }
    }

    #[test]
    fn a_noop_filter_fetches_exactly_the_limit() {
        let mut asked = Vec::new();
        let out = ResultFilter::default()
            .fetch(
                5,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, _| false,
            )
            .unwrap();
        assert_eq!(asked, vec![5]);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn the_window_widens_until_the_limit_is_filled() {
        // Only multiples of 20 pass; the first window (limit 2 -> 12) holds
        // one, the next (48) holds three.
        let mut asked = Vec::new();
        let out = filter(Some(0.5))
            .fetch(
                2,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, r| r % 20 == 0,
            )
            .unwrap();
        assert_eq!(asked, vec![12, 48]);
        assert_eq!(out, vec![0, 20]);
    }

    #[test]
    fn it_stops_when_the_candidates_run_out() {
        let mut asked = Vec::new();
        let out = filter(Some(0.5))
            .fetch(
                5,
                |n| {
                    asked.push(n);
                    Ok((0..7usize.min(n)).collect::<Vec<usize>>())
                },
                |_, r| *r == 3,
            )
            .unwrap();
        assert_eq!(asked, vec![15]);
        assert_eq!(out, vec![3]);
    }

    #[test]
    fn the_window_is_capped() {
        let mut asked = Vec::new();
        let out: Vec<usize> = filter(Some(0.5))
            .fetch(
                10,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect())
                },
                |_, _| false,
            )
            .unwrap();
        assert!(out.is_empty());
        assert_eq!(*asked.last().unwrap(), MAX_WINDOW);
    }
}
