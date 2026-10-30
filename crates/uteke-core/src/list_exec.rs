//! Running a decoded list request (#1439, PR D): the choice between a plain
//! page, a point-in-time page and the pagination envelope, once, for HTTP and
//! MCP. The behaviour is the HTTP handler's, moved here unchanged.

use crate::memory::types::Memory;
use crate::{Error, ListPageMeta, ListRequest, Uteke};

/// What a list produced, before a surface renders it.
#[derive(Debug)]
pub enum ListOutput {
    /// The page of memories (the default response).
    Memories(Vec<Memory>),
    /// The page plus its pagination metadata (`include_meta`, not with `at`).
    Page {
        memories: Vec<Memory>,
        total: usize,
        meta: ListPageMeta,
    },
}

impl Uteke {
    /// Run `req` as `POST /list` always has.
    pub fn run_list(&self, req: &ListRequest) -> Result<ListOutput, Error> {
        let namespace = req.namespace.as_deref();
        let memories = match req.at {
            Some(pit) => {
                self.list_at_time(req.tag.as_deref(), req.limit, req.offset, namespace, pit)?
            }
            None => self.list(req.tag.as_deref(), req.limit, req.offset, namespace)?,
        };
        if !req.wants_envelope() {
            return Ok(ListOutput::Memories(memories));
        }
        // Total matching rows (same filters as the page). A failed count is 0,
        // as it always was.
        let total = match &req.tag {
            Some(tag) => self.count_by_tag(tag, namespace),
            None => self.count(namespace),
        }
        .unwrap_or(0);
        let meta = req.page_meta(total, memories.len());
        Ok(ListOutput::Page {
            memories,
            total,
            meta,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ListInput;
    use crate::test_support::open_keyword;

    fn req(f: impl FnOnce(&mut ListInput)) -> ListRequest {
        let mut i = ListInput::default();
        f(&mut i);
        ListRequest::decode(i).unwrap()
    }

    fn store_with(n: usize) -> Uteke {
        let u = open_keyword();
        for i in 0..n {
            u.remember(&format!("memory number {i} about rust"), &["t"], None, None)
                .unwrap();
        }
        u
    }

    #[test]
    fn the_default_is_a_bare_page() {
        let u = store_with(3);
        let ListOutput::Memories(m) = u.run_list(&req(|_| {})).unwrap() else {
            panic!("bare page expected");
        };
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn include_meta_adds_the_total_and_the_next_offset() {
        let u = store_with(5);
        let out = u
            .run_list(&req(|i| {
                i.limit = Some(2);
                i.include_meta = true;
            }))
            .unwrap();
        let ListOutput::Page {
            memories,
            total,
            meta,
        } = out
        else {
            panic!("envelope expected");
        };
        assert_eq!((memories.len(), total), (2, 5));
        assert!(meta.has_more);
        assert_eq!(meta.next_offset, Some(2));
        // The last page has no next offset.
        let ListOutput::Page { meta, .. } = u
            .run_list(&req(|i| {
                i.limit = Some(2);
                i.offset = Some(4);
                i.include_meta = true;
            }))
            .unwrap()
        else {
            panic!()
        };
        assert!(!meta.has_more && meta.next_offset.is_none());
    }

    #[test]
    fn include_meta_is_ignored_with_a_point_in_time() {
        let u = store_with(2);
        let out = u
            .run_list(&req(|i| {
                i.include_meta = true;
                i.at = Some("2999-01-01T00:00:00Z".into());
            }))
            .unwrap();
        assert!(matches!(out, ListOutput::Memories(m) if m.len() == 2));
        let before = u
            .run_list(&req(|i| i.at = Some("2000-01-01T00:00:00Z".into())))
            .unwrap();
        assert!(matches!(before, ListOutput::Memories(m) if m.is_empty()));
    }
}
