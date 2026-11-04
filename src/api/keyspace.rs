//! Public keyspace boundaries. Internal metadata is not client data.

use crate::{LsmEngine, Result};
use actix_web::HttpResponse;

type Rows = Vec<(Vec<u8>, Vec<u8>)>;
type Page = (Rows, Option<String>);

pub(crate) fn is_reserved(key: &[u8]) -> bool {
    key.starts_with(b"__")
}

pub(crate) fn rejected() -> HttpResponse {
    HttpResponse::BadRequest().json(serde_json::json!({"error":"reserved key prefix"}))
}

pub(crate) fn scan(
    engine: &LsmEngine,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    limit: usize,
) -> Result<Rows> {
    collect(engine, lower, upper, limit, usize::MAX, |_, _| true)
}

pub(crate) fn scan_matching(
    engine: &LsmEngine,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    limit: usize,
    matches: impl Fn(&[u8], &[u8]) -> bool,
) -> Result<Rows> {
    collect(engine, lower, upper, limit.min(100), 10_000, matches)
}

fn collect(
    engine: &LsmEngine,
    lower: Option<&[u8]>,
    upper: Option<&[u8]>,
    limit: usize,
    scan_budget: usize,
    matches: impl Fn(&[u8], &[u8]) -> bool,
) -> Result<Rows> {
    let limit = limit.min(crate::core::engine::MAX_SCAN_LIMIT + 1);
    let mut output = Vec::new();
    let mut cursor = lower.map(<[u8]>::to_vec);
    let mut examined = 0;
    while output.len() < limit && examined < scan_budget {
        let page_size = (scan_budget - examined).min(256);
        let page = engine.scan_cf("default", cursor.as_deref(), upper, Some(page_size))?;
        let exhausted = page.len() < page_size;
        examined += page.len();
        let Some((last, _)) = page.last() else { break };
        // The smallest byte key strictly greater than the inclusive page end.
        let mut next = last.clone();
        next.push(0);
        for (key, value) in page {
            if !is_reserved(&key) && matches(&key, &value) {
                output.push((key, value));
                if output.len() == limit {
                    break;
                }
            }
        }
        if exhausted {
            break;
        }
        cursor = Some(next);
    }
    Ok(output)
}

pub(crate) fn prefix(
    engine: &LsmEngine,
    prefix: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Page> {
    let limit = limit.min(crate::core::engine::MAX_SCAN_LIMIT);
    let mut lower = prefix.as_bytes().to_vec();
    if let Some(cursor) = cursor {
        let mut after = cursor.as_bytes().to_vec();
        after.push(0);
        lower = lower.max(after);
    }
    let upper = LsmEngine::prefix_end(prefix);
    let mut rows = scan(engine, Some(&lower), upper.as_deref(), limit + 1)?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = if more {
        rows.last()
            .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
    } else {
        None
    };
    Ok((rows, next))
}
