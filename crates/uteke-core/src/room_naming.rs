//! Room id naming convention `category:title` (#1715).
//!
//! The owner's convention: a room id has the shape `category:title`, for example
//! `project:uteke` or `research:docker`. Only the SHAPE is the rule; the
//! category itself is a recommendation, not a closed list, so a custom category
//! of the right shape is accepted at every stage.
//!
//! Stage 1 (this module): creating or renaming a room to an id that is not of
//! that shape still succeeds, with a warning. Reads, recall and existing rooms
//! are never warned or blocked. Stages 2 (migrate old rooms through aliases) and
//! 3 (reject new ids of the wrong shape) are separate steps.

/// Categories recommended for a room id. Not enforced.
pub const RECOMMENDED_ROOM_CATEGORIES: [&str; 5] = ["project", "research", "team", "ops", "ref"];

/// The longest room id the convention allows.
pub const MAX_ROOM_ID_LEN: usize = 64;

/// True when `id` has the shape `category:title`: lowercase, exactly one `:`,
/// `^[a-z][a-z0-9-]*:[a-z0-9][a-z0-9._-]*$`, at most [`MAX_ROOM_ID_LEN`] bytes.
pub fn is_conventional_room_id(id: &str) -> bool {
    if id.len() > MAX_ROOM_ID_LEN {
        return false;
    }
    let Some((category, title)) = id.split_once(':') else {
        return false;
    };
    if title.contains(':') {
        return false;
    }
    let mut c = category.bytes();
    let category_ok = matches!(c.next(), Some(b'a'..=b'z'))
        && c.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    let mut t = title.bytes();
    let title_ok = matches!(t.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && t.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'));
    category_ok && title_ok
}

/// The stage-1 warning for a room id that is created or renamed to, or `None`
/// when the id has the conventional shape.
pub fn room_id_warning(id: &str) -> Option<String> {
    if is_conventional_room_id(id) {
        return None;
    }
    Some(format!(
        "Room id '{id}' is not of the form category:title (lowercase, one colon, at most \
         {MAX_ROOM_ID_LEN} characters; for example project:uteke or research:docker; \
         recommended categories: {}). It was accepted; ids of this form will be required later.",
        RECOMMENDED_ROOM_CATEGORIES.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conventional_shape_is_accepted_with_any_category() {
        for id in [
            "project:uteke",
            "research:docker",
            "team:weekly.standup",
            "ops:db_backup-2026",
            "lab:xyz",
            "a:b",
            "x-1:0",
        ] {
            assert!(is_conventional_room_id(id), "{id}");
            assert_eq!(room_id_warning(id), None, "{id}");
        }
    }

    #[test]
    fn ids_of_another_shape_are_flagged() {
        for id in [
            "uteke",
            "uteke-conventions",
            "",
            ":title",
            "category:",
            "Project:uteke",
            "project:Uteke",
            "project:uteke:extra",
            "1project:uteke",
            "-project:uteke",
            "proj_ect:uteke",
            "project:-uteke",
            "project:.uteke",
            "project:ute ke",
            "project:utéke",
        ] {
            assert!(!is_conventional_room_id(id), "{id:?}");
            let w = room_id_warning(id).unwrap_or_else(|| panic!("no warning for {id:?}"));
            assert!(
                w.contains("category:title") && w.contains("accepted"),
                "{w}"
            );
        }
    }

    #[test]
    fn the_length_limit_is_64_bytes_inclusive() {
        let at_limit = format!("a:{}", "b".repeat(MAX_ROOM_ID_LEN - 2));
        assert_eq!(at_limit.len(), MAX_ROOM_ID_LEN);
        assert!(is_conventional_room_id(&at_limit));
        let over = format!("a:{}", "b".repeat(MAX_ROOM_ID_LEN - 1));
        assert!(!is_conventional_room_id(&over));
    }

    #[test]
    fn the_warning_names_the_recommended_categories() {
        let w = room_id_warning("plain").unwrap();
        for c in RECOMMENDED_ROOM_CATEGORIES {
            assert!(w.contains(c), "{w}");
        }
    }
}
