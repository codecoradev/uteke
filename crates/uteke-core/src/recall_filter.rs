//! Result filters of a recall: `strict` on similarity (#1650) and `min_trust`
//! on the provenance tier (#1651).
//!
//! Both are applied to the ranked candidates BEFORE the result is cut to the
//! caller's limit, so a filtered recall still returns up to `limit` rows when
//! that many qualify. The strategies cut to the limit themselves, so
//! [`ResultFilter::fetch`] asks for a larger window and widens it until enough
//! rows survive or the candidates run out.
//!
//! Neither filter changes `score` or the ranking.

use crate::Uteke;
use crate::error::Error;
use crate::memory::types::{SearchResult, UnifiedSearchResult};
use crate::provenance::TrustTier;

/// Largest candidate window [`ResultFilter::fetch`] asks a strategy for.
const MAX_WINDOW: usize = 1000;

/// The filters a recall request carries next to its ranking parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResultFilter {
    /// `strict`: keep a hit only when its `similarity` (the vector-leg cosine)
    /// is at least this. A hit without a similarity is dropped.
    pub min_similarity: Option<f32>,
    /// Keep a hit only when its provenance tier is at least this.
    ///
    /// This is the provenance TIER (author_type + source_type), not a trust
    /// state, and `author_type` is self-declared: a convenience filter, not a
    /// security boundary.
    pub min_trust: Option<TrustTier>,
}

impl ResultFilter {
    /// No filter is active: results pass through untouched.
    pub fn is_noop(&self) -> bool {
        self.min_similarity.is_none() && self.min_trust.is_none()
    }

    fn keep_similarity(&self, similarity: Option<f32>) -> bool {
        match self.min_similarity {
            None => true,
            Some(floor) => similarity.is_some_and(|s| s >= floor),
        }
    }

    /// Whether a memory result passes both filters.
    pub fn keep_memory(&self, r: &SearchResult) -> bool {
        self.keep_similarity(r.similarity)
            && self
                .min_trust
                .is_none_or(|min| TrustTier::of(&r.memory) >= min)
    }

    /// Whether a unified result passes both filters. A document has neither a
    /// similarity nor a provenance tier, so any active filter drops it.
    pub fn keep_unified(&self, r: &UnifiedSearchResult) -> bool {
        self.keep_similarity(r.similarity)
            && self.min_trust.is_none_or(|min| r.trust_tier >= Some(min))
    }

    /// Run `fetch(window)` and keep the rows `keep` accepts, up to `limit`.
    ///
    /// With no active filter this is exactly `fetch(limit)`. Otherwise the
    /// window starts at three times the limit and grows fourfold while the
    /// strategy keeps returning a full window but too few rows pass, up to
    /// [`MAX_WINDOW`] candidates. A pass that keeps nothing goes straight to
    /// [`MAX_WINDOW`], so a query no hit satisfies costs two passes, not five.
    ///
    /// A recall counts what it returns (access count, hot boost). The widened
    /// passes return candidates that are then discarded, so they run with the
    /// touch held, and only the rows finally returned are touched, once.
    /// `id_of` names the memory behind a row (`None` for a document).
    pub fn fetch<T>(
        &self,
        uteke: &Uteke,
        limit: usize,
        mut fetch: impl FnMut(usize) -> Result<Vec<T>, Error>,
        keep: impl Fn(&Self, &T) -> bool,
        id_of: impl Fn(&T) -> Option<&str>,
    ) -> Result<Vec<T>, Error> {
        if self.is_noop() {
            return fetch(limit);
        }
        let _hold = TouchHold::new(uteke);
        let mut window = limit.saturating_mul(3).max(limit + 10).min(MAX_WINDOW);
        let kept = loop {
            let raw = fetch(window)?;
            let exhausted = raw.len() < window;
            let mut kept: Vec<T> = raw.into_iter().filter(|r| keep(self, r)).collect();
            if kept.len() >= limit || exhausted || window >= MAX_WINDOW {
                kept.truncate(limit);
                break kept;
            }
            // A first pass that keeps nothing says the query is off-topic for
            // the filter: widen once, straight to the largest window, instead
            // of re-running the strategy at 4x each time (#1671). The rows
            // that finally pass are the same, the first `limit` of them.
            window = if kept.is_empty() {
                MAX_WINDOW
            } else {
                window.saturating_mul(4).min(MAX_WINDOW)
            };
        };
        drop(_hold);
        let ids: Vec<&str> = kept.iter().filter_map(&id_of).collect();
        uteke.touch_ids(&ids);
        Ok(kept)
    }
}

/// Holds [`Uteke::touch_hold`] for its lifetime, also on an early return.
struct TouchHold<'a>(&'a Uteke);

impl<'a> TouchHold<'a> {
    fn new(uteke: &'a Uteke) -> Self {
        uteke
            .touch_hold
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(uteke)
    }
}

impl Drop for TouchHold<'_> {
    fn drop(&mut self) {
        self.0
            .touch_hold
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(sim: Option<f32>, trust: Option<TrustTier>) -> ResultFilter {
        ResultFilter {
            min_similarity: sim,
            min_trust: trust,
        }
    }

    #[test]
    fn a_noop_filter_fetches_exactly_the_limit() {
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out = ResultFilter::default()
            .fetch(
                &u,
                5,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, _| false,
                |_| None,
            )
            .unwrap();
        assert_eq!(asked, vec![5]);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn the_window_widens_until_the_limit_is_filled() {
        // Only multiples of 20 pass; the first window (limit 2 -> 12) holds
        // one, the next (48) holds three.
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out = filter(Some(0.5), None)
            .fetch(
                &u,
                2,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, r| r % 20 == 0,
                |_| None,
            )
            .unwrap();
        assert_eq!(asked, vec![12, 48]);
        assert_eq!(out, vec![0, 20]);
    }

    #[test]
    fn it_stops_when_the_candidates_run_out() {
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out = filter(Some(0.5), None)
            .fetch(
                &u,
                5,
                |n| {
                    asked.push(n);
                    Ok((0..7usize.min(n)).collect::<Vec<usize>>())
                },
                |_, r| *r == 3,
                |_| None,
            )
            .unwrap();
        assert_eq!(asked, vec![15]);
        assert_eq!(out, vec![3]);
    }

    #[test]
    fn the_window_is_capped() {
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out: Vec<usize> = filter(Some(0.5), None)
            .fetch(
                &u,
                10,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect())
                },
                |_, _| false,
                |_| None,
            )
            .unwrap();
        assert!(out.is_empty());
        assert_eq!(*asked.last().unwrap(), MAX_WINDOW);
    }

    #[test]
    fn a_pass_that_keeps_nothing_widens_once_to_the_maximum() {
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out: Vec<usize> = filter(Some(0.5), None)
            .fetch(
                &u,
                5,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect())
                },
                |_, _| false,
                |_| None,
            )
            .unwrap();
        assert!(out.is_empty());
        assert_eq!(asked, vec![15, MAX_WINDOW], "two passes, not five");
    }

    #[test]
    fn a_late_hit_is_still_found_after_an_empty_first_pass() {
        // Nothing in the first 15 passes, one at rank 500: the jump to the
        // maximum window finds it, like the fourfold steps did.
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out = filter(Some(0.5), None)
            .fetch(
                &u,
                5,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, r| *r == 500 || *r == 700,
                |_| None,
            )
            .unwrap();
        assert_eq!(out, vec![500, 700]);
        assert_eq!(asked, vec![15, MAX_WINDOW]);
    }

    #[test]
    fn a_partly_filled_first_pass_still_widens_in_steps() {
        let u = crate::test_support::open_keyword();
        let mut asked = Vec::new();
        let out = filter(Some(0.5), None)
            .fetch(
                &u,
                5,
                |n| {
                    asked.push(n);
                    Ok((0..n).collect::<Vec<usize>>())
                },
                |_, r| r % 10 == 0,
                |_| None,
            )
            .unwrap();
        assert_eq!(out, vec![0, 10, 20, 30, 40]);
        assert_eq!(asked, vec![15, 60]);
    }
}
