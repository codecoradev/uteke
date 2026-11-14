//! Skill library handlers (#1731): search, list, show and similar. Read-only;
//! `index`, `approve` and `record` are CLI-only. Every route validates through
//! `Uteke::skill_remote`, the same entry point as the MCP tools.

use super::*;
use uteke_core::skill_remote::{SkillRemoteArgs, SkillRemoteOp};

fn skill_route<R>(c: &mut Call, op: SkillRemoteOp) -> Response<Cursor<Vec<u8>>>
where
    R: serde::de::DeserializeOwned + Into<SkillRemoteArgs>,
{
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let args: SkillRemoteArgs = match read_body::<R>(req.as_reader()) {
        Ok(r) => r.into(),
        Err(e) => return ctx.error_response_for(req, 400, e),
    };
    match uteke.skill_remote(op, &args) {
        Ok(answer) => ctx.ok_response_for(req, &answer),
        Err(e) => ctx.core_error_embedding_503(
            req,
            &e,
            "embedding model not available for this skill request",
        ),
    }
}

/// POST /skill/search
pub(super) fn post_skill_search(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    skill_route::<SkillSearchRequest>(c, SkillRemoteOp::Search)
}

/// POST /skill/list
pub(super) fn post_skill_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    skill_route::<SkillListRequest>(c, SkillRemoteOp::List)
}

/// POST /skill/show
pub(super) fn post_skill_show(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    skill_route::<SkillShowRequest>(c, SkillRemoteOp::Show)
}

/// POST /skill/similar
pub(super) fn post_skill_similar(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    skill_route::<SkillSimilarRequest>(c, SkillRemoteOp::Similar)
}
