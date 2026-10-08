//! List, Get, and Stats commands.

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

/// Rows fetched per round trip while filtering client-side.
const FILTER_PAGE_SIZE: usize = 200;

/// Return the matches `[offset, offset + limit)` of a stream that can only be
/// filtered after it is fetched. `fetch(offset, page_size)` returns raw rows
/// starting at the given RAW offset; a short or empty page ends the scan.
fn collect_filtered_page<T>(
    offset: usize,
    limit: usize,
    page_size: usize,
    mut fetch: impl FnMut(usize, usize) -> Result<Vec<T>, String>,
    keep: impl Fn(&T) -> bool,
) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    if limit == 0 {
        return Ok(out);
    }
    let (mut raw_offset, mut skipped) = (0usize, 0usize);
    loop {
        let page = fetch(raw_offset, page_size)?;
        let n = page.len();
        for row in page {
            if !keep(&row) {
                continue;
            }
            if skipped < offset {
                skipped += 1;
            } else {
                out.push(row);
                if out.len() == limit {
                    return Ok(out);
                }
            }
        }
        raw_offset += n;
        if n < page_size {
            return Ok(out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_list(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    tag: &Option<String>,
    limit: usize,
    offset: usize,
    entity: Option<&str>,
    category: Option<&str>,
    at: Option<&str>,
) -> Result<(), String> {
    tracing::info!(
        "Listing memories (tag: {:?}, limit: {limit}, offset: {offset})",
        tag
    );

    // Time-travel mode: parse --at as RFC3339 and use list_at_time
    let point_in_time = match at {
        Some(at_str) => Some(
            chrono::DateTime::parse_from_rfc3339(at_str)
                .map_err(|e| {
                    format!(
                        "Invalid --at timestamp: {e}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                    )
                })?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    let fetch =
        |off: usize, lim: usize| -> Result<Vec<uteke_core::memory::types::Memory>, String> {
            match point_in_time {
                Some(pit) => uteke
                    .list_at_time(tag.as_deref(), lim, off, ns, pit)
                    .map_err(|e| format!("Failed to list at time: {e}")),
                None => uteke
                    .list(tag.as_deref(), lim, off, ns)
                    .map_err(|e| format!("Failed to list: {e}")),
            }
        };

    let results = if entity.is_some() || category.is_some() {
        // Entity/category live in the metadata JSON and cannot be filtered in
        // SQL, so page through the store and apply `--limit/--offset` to the
        // MATCHES. Filtering a single already-paginated page (the old
        // behaviour) returned fewer than `limit` rows and made `--offset`
        // skip rows that were never candidates (#1332).
        let meta_is = |m: &uteke_core::memory::types::Memory, key: &str, want: &str| {
            m.metadata
                .get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == want)
        };
        collect_filtered_page(offset, limit, FILTER_PAGE_SIZE, fetch, |m| {
            entity.is_none_or(|e| meta_is(m, "entity", e))
                && category.is_none_or(|c| meta_is(m, "category", c))
        })?
    } else {
        fetch(offset, limit)?
    };

    if cli.json {
        output::print_json(&results);
    } else {
        output::print_list_human(&results);
    }
    Ok(())
}

pub(crate) fn run_get(cli: &Cli, uteke: &Uteke, id: &str) -> Result<(), String> {
    tracing::info!("Getting memory: {id}");
    let memory = uteke
        .get(id)
        .map_err(|e| format!("Failed to get memory: {e}"))?;
    if cli.json {
        output::print_json(&memory);
    } else {
        output::print_get_human(&memory);
    }
    Ok(())
}

pub(crate) fn run_stats(cli: &Cli, uteke: &Uteke, ns: Option<&str>) -> Result<(), String> {
    tracing::info!("Getting stats");
    let stats = uteke
        .stats(ns)
        .map_err(|e| format!("Failed to get stats: {e}"))?;
    if cli.json {
        output::print_json(&stats);
    } else {
        output::print_stats_human(&stats);
    }
    Ok(())
}

#[cfg(test)]
mod filtered_page_tests {
    use super::collect_filtered_page;

    /// A raw store of 0..100 where only multiples of 7 match.
    fn fetch(off: usize, lim: usize) -> Result<Vec<u32>, String> {
        Ok((0u32..100).skip(off).take(lim).collect())
    }
    fn keep(n: &u32) -> bool {
        n % 7 == 0
    }

    #[test]
    fn limit_and_offset_apply_to_matches_not_raw_rows() {
        // Matches: 0,7,14,...,98 (15 of them).
        let got = collect_filtered_page(0, 5, 10, fetch, keep).unwrap();
        assert_eq!(got, vec![0, 7, 14, 21, 28], "limit counts matches");
        let got = collect_filtered_page(5, 5, 10, fetch, keep).unwrap();
        assert_eq!(got, vec![35, 42, 49, 56, 63], "offset skips matches");
    }

    #[test]
    fn pages_through_the_whole_store_and_stops() {
        let got = collect_filtered_page(0, 1000, 10, fetch, keep).unwrap();
        assert_eq!(got.len(), 15);
        assert_eq!(*got.last().unwrap(), 98);
        let got = collect_filtered_page(14, 5, 10, fetch, keep).unwrap();
        assert_eq!(got, vec![98], "only one match left after the offset");
        assert!(
            collect_filtered_page(100, 5, 10, fetch, keep)
                .unwrap()
                .is_empty()
        );
        assert!(
            collect_filtered_page(0, 0, 10, fetch, keep)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fetch_errors_propagate() {
        let err = collect_filtered_page::<u32>(0, 3, 10, |_, _| Err("boom".into()), |_| true)
            .unwrap_err();
        assert_eq!(err, "boom");
    }
}
