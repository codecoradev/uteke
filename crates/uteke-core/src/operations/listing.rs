//! Listing and namespace management: list, get, namespace counts, move, rename and delete.

use super::*;

impl crate::Uteke {
    /// List memories with optional tag filter and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        self.store.list(tag, namespace, limit, offset)
    }

    /// Get a single memory by ID.
    pub fn get(&self, id: &str) -> Result<Memory, Error> {
        let memory = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::db_msg(format!("Memory not found: {id}")))?;
        self.store.touch_access(id).ok();
        Ok(memory)
    }

    /// List all namespaces.
    pub fn list_namespaces(&self) -> Result<Vec<String>, Error> {
        self.store.list_namespaces()
    }

    /// List all namespaces with memory counts (#527).
    ///
    /// Returns `[(namespace, count)]` — e.g. `[("default", 432), ("cto", 28)]`.
    pub fn list_namespaces_with_counts(&self) -> Result<Vec<(String, usize)>, Error> {
        self.store.list_namespaces_with_counts()
    }

    /// List all namespaces with counts split by lifecycle state (#1181).
    ///
    /// Returns `[(namespace, active, deprecated)]` so clients can show honest
    /// breakdowns — deprecated-only "ghost" namespaces look identical to
    /// active ones in plain counts.
    pub fn list_namespaces_with_lifecycle_counts(
        &self,
    ) -> Result<Vec<(String, usize, usize)>, Error> {
        self.store.list_namespaces_with_lifecycle_counts()
    }

    /// Move a single memory to another namespace (#1181).
    ///
    /// Returns `Ok(false)` when the memory does not exist. Namespace is a
    /// plain column and embeddings are content-based, so no re-embed happens.
    pub fn move_memory(&self, id: &str, namespace: &str) -> Result<bool, Error> {
        Self::validate_namespace_name(namespace)?;
        let existing = match self.store.get_by_id(id)? {
            Some(memory) => memory,
            None => return Ok(false),
        };
        let moved =
            self.store
                .move_memory_namespace(id, namespace, &chrono::Utc::now().to_rfc3339())?;
        if moved {
            self.recall_cache.invalidate_namespace(&existing.namespace);
            self.recall_cache.invalidate_namespace(namespace);
        }
        Ok(moved)
    }

    /// Rename a namespace, merging into the target when it exists (#1181).
    ///
    /// Single atomic `UPDATE` — the old name vanishes naturally because
    /// namespaces are a derived view over `memories.namespace`.
    pub fn rename_namespace(&self, from: &str, to: &str) -> Result<NamespaceRenameResult, Error> {
        Self::validate_namespace_name(from)?;
        Self::validate_namespace_name(to)?;
        if from == to {
            return Err(Error::Validation(
                "Rename source and target namespaces are identical".to_string(),
            ));
        }
        let (active, deprecated, total) = self.store.namespace_lifecycle_counts(from)?;
        if total == 0 {
            return Err(Error::not_found(format!("Namespace not found: {from}")));
        }
        let target_existed = self.store.namespace_lifecycle_counts(to)?.2 > 0;
        let moved = self
            .store
            .rename_namespace(from, to, &chrono::Utc::now().to_rfc3339())?;
        self.recall_cache.invalidate_namespace(from);
        self.recall_cache.invalidate_namespace(to);
        tracing::info!(
            "Namespace rename/merge: '{from}' -> '{to}' moved {moved} memories \
             ({active} active, {deprecated} deprecated, target_existed={target_existed})"
        );
        Ok(NamespaceRenameResult {
            from: from.to_string(),
            to: to.to_string(),
            moved,
            target_existed,
        })
    }

    /// Delete a namespace with an explicit strategy for its memories (#1181).
    ///
    /// Namespaces are derived — the name only disappears once no memory
    /// references it, so deletion must first decide the fate of its memories:
    /// - `refuse`: fail while any memory (incl. deprecated) uses the name.
    /// - `merge`: move all memories into `target` (the name disappears).
    /// - `deprecate`: soft-delete all memories (recycle bin + TTL) — the name
    ///   stays visible as a deprecated-only ghost with honest counts.
    ///
    /// There is no hard-delete path, consistent with the lifecycle design.
    pub fn delete_namespace(
        &self,
        name: &str,
        strategy: &str,
        target: Option<&str>,
    ) -> Result<NamespaceDeleteResult, Error> {
        Self::validate_namespace_name(name)?;
        match strategy {
            "refuse" | "merge" | "deprecate" => {}
            other => {
                return Err(Error::Validation(format!(
                    "Unknown delete strategy '{other}'. Expected: refuse, merge, deprecate"
                )));
            }
        }
        let (active, deprecated, total) = self.store.namespace_lifecycle_counts(name)?;
        let no_op = |empty: bool| NamespaceDeleteResult {
            name: name.to_string(),
            strategy: strategy.to_string(),
            affected: 0,
            target: target.map(str::to_string),
            empty,
        };
        if total == 0 {
            // Namespace is empty / already gone — idempotent no-op.
            return Ok(no_op(true));
        }
        match strategy {
            "merge" => {
                let target = target.ok_or_else(|| {
                    Error::Validation("strategy=merge requires a 'target' namespace".to_string())
                })?;
                Self::validate_namespace_name(target)?;
                if target == name {
                    return Err(Error::Validation(
                        "Merge target must differ from the deleted namespace".to_string(),
                    ));
                }
                let moved =
                    self.store
                        .rename_namespace(name, target, &chrono::Utc::now().to_rfc3339())?;
                self.recall_cache.invalidate_namespace(name);
                self.recall_cache.invalidate_namespace(target);
                tracing::info!(
                    "Namespace delete (merge): '{name}' -> '{target}' moved {moved} memories"
                );
                Ok(NamespaceDeleteResult {
                    name: name.to_string(),
                    strategy: strategy.to_string(),
                    affected: moved,
                    target: Some(target.to_string()),
                    empty: true,
                })
            }
            "deprecate" => {
                let reason = format!("namespace delete (strategy=deprecate) of '{name}' (#1181)");
                let affected = self.store.deprecate_by_namespace(name, &reason)?;
                let ids = self.store.namespace_ids(name)?;
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during delete_namespace"))?;
                index_sync::remove_ids(
                    &mut *index,
                    ids.iter().map(String::as_str),
                    &SyncPolicy::STANDARD,
                )
                .into_delete_result("delete_namespace (deprecate)")?;
                self.recall_cache.invalidate_namespace(name);
                tracing::info!(
                    "Namespace delete (deprecate): '{name}' soft-deleted {affected} memories \
                     ({deprecated} were already deprecated)"
                );
                Ok(NamespaceDeleteResult {
                    name: name.to_string(),
                    strategy: strategy.to_string(),
                    affected,
                    target: None,
                    // The name survives as a deprecated-only ghost listing.
                    empty: false,
                })
            }
            // `refuse` — validated above; total > 0 always refuses.
            _ => Err(Error::Validation(format!(
                "Namespace '{name}' still holds {total} memory(ies) \
                 ({active} active, {deprecated} deprecated); refusing to delete. \
                 Use strategy=merge (with target) or strategy=deprecate."
            ))),
        }
    }

    /// Shared namespace name validation (#1181).
    fn validate_namespace_name(name: &str) -> Result<(), Error> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::Validation(
                "Namespace name must not be empty".to_string(),
            ));
        }
        if trimmed.len() > 128 {
            return Err(Error::Validation(format!(
                "Namespace name too long ({} > 128 chars)",
                trimmed.len()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod namespace_management_tests {
    use crate::Uteke;

    /// #1181: move_memory updates the namespace column and both the old and
    /// the new namespace disappear/appear correctly in listings.
    #[test]
    fn move_memory_updates_namespace() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("move me", &[], None, Some("alpha"))
            .expect("remember");

        let moved = u.move_memory(&id, "beta").expect("move");
        assert!(moved);
        let mem = u.get_by_id(&id).expect("get").expect("exists");
        assert_eq!(mem.namespace, "beta");

        let listings = u.list_namespaces().expect("list");
        assert!(
            !listings.contains(&"alpha".to_string()),
            "old name vanishes"
        );
        assert!(listings.contains(&"beta".to_string()));

        // Unknown ID → Ok(false), not an error.
        let missing = u.move_memory("00000000-0000-4000-8000-000000000000", "beta");
        assert!(matches!(missing, Ok(false)));
    }

    /// #1181: rename moves all memories; renaming onto an existing namespace
    /// merges and reports `target_existed = true`.
    #[test]
    fn rename_namespace_moves_and_merges() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        u.remember("a one", &[], None, Some("old")).expect("a1");
        u.remember("a two", &[], None, Some("old")).expect("a2");
        u.remember("b one", &[], None, Some("new")).expect("b1");

        // Merge path: target exists.
        let merge = u.rename_namespace("old", "new").expect("merge");
        assert!(merge.target_existed);
        assert_eq!(merge.moved, 2);
        assert_eq!(merge.from, "old");
        assert_eq!(merge.to, "new");

        let listings = u.list_namespaces().expect("list");
        assert!(!listings.contains(&"old".to_string()));
        assert!(listings.contains(&"new".to_string()));

        // Plain rename path: target does not exist.
        u.remember("c one", &[], None, Some("temp")).expect("c1");
        let plain = u.rename_namespace("temp", "final").expect("rename");
        assert!(!plain.target_existed);
        assert_eq!(plain.moved, 1);

        // Same-name rename is rejected.
        assert!(u.rename_namespace("final", "final").is_err());
        // Unknown source namespace is rejected.
        assert!(u.rename_namespace("ghost", "anywhere").is_err());
    }

    /// #1181: delete strategies — refuse (default) blocks while memories
    /// remain; merge moves everything away; deprecate soft-deletes without
    /// hard-deleting anything.
    #[test]
    fn delete_namespace_strategies() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id1 = u.remember("d one", &[], None, Some("temp-ns")).expect("d1");
        let id2 = u.remember("d two", &[], None, Some("temp-ns")).expect("d2");

        // refuse (default): blocked while memories exist.
        let refused = u.delete_namespace("temp-ns", "refuse", None);
        assert!(refused.is_err(), "refuse must block a non-empty namespace");

        // merge: moves all memories into the target, name disappears.
        let merged = u
            .delete_namespace("temp-ns", "merge", Some("archive"))
            .expect("merge delete");
        assert_eq!(merged.strategy, "merge");
        assert_eq!(merged.affected, 2);
        assert_eq!(merged.target.as_deref(), Some("archive"));
        assert!(merged.empty);
        let listings = u.list_namespaces().expect("list");
        assert!(!listings.contains(&"temp-ns".to_string()));
        assert!(u.get_by_id(&id1).expect("get").expect("survives").namespace == "archive");

        // deprecate: soft-delete only, no hard delete; the name remains as a
        // deprecated-only ghost with honest lifecycle counts.
        let dep1 = u
            .remember("e one", &[], None, Some("ghost-ns"))
            .expect("e1");
        let dep = u
            .delete_namespace("ghost-ns", "deprecate", None)
            .expect("deprecate delete");
        assert_eq!(dep.strategy, "deprecate");
        assert_eq!(dep.affected, 1);
        assert!(!dep.empty);
        let mem = u.get_by_id(&dep1).expect("get").expect("still stored");
        assert!(mem.deprecated, "memory must be soft-deleted, not removed");
        let lifecycle = u
            .list_namespaces_with_lifecycle_counts()
            .expect("lifecycle counts");
        let ghost = lifecycle
            .iter()
            .find(|(name, _, _)| name == "ghost-ns")
            .expect("ghost namespace stays listed");
        assert_eq!((ghost.1, ghost.2), (0, 1), "0 active / 1 deprecated");
        assert!(
            u.get_by_id(&id2).is_ok(),
            "nothing from the earlier merge path was lost"
        );

        // Unknown strategy is rejected.
        assert!(u.delete_namespace("ghost-ns", "hard-delete", None).is_err());
    }

    /// #1172 Fase 1: remember records a SHA-256 source hash; provenance()
    /// returns the full chain (fields + tier + timeline) and detects content
    /// modified after write via hash mismatch.
    #[test]
    fn provenance_chain_and_source_hash() {
        use sha2::Digest;

        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("the deploy window is 09:00 WIB", &[], None, Some("ops"))
            .expect("remember");

        let report = u.provenance(&id).expect("provenance").expect("exists");
        assert_eq!(report.id, id);
        assert_eq!(report.namespace, "ops");
        assert_eq!(report.author_type, "agent");
        // Hash at write time must match a live recomputation.
        let expected: String = sha2::Sha256::digest(report.content.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(report.source_hash.as_deref(), Some(expected.as_str()));
        assert_eq!(report.content_hash_now, expected);
        assert!(!report.events.is_empty(), "Created event must be recorded");
        assert!(report.events.iter().any(|e| e.event_type == "created"));

        // Update content without touching source_hash → mismatch detected
        // (this is the tamper-evidence property).
        u.store
            .conn
            .execute(
                "UPDATE memories SET content = 'tampered' WHERE id = ?1",
                rusqlite::params![id],
            )
            .expect("tamper");
        let after = u.provenance(&id).expect("provenance").expect("exists");
        assert_eq!(after.content, "tampered");
        assert_ne!(
            after.source_hash.as_deref(),
            Some(after.content_hash_now.as_str()),
            "post-write modification must break the hash match"
        );

        // Unknown ID → Ok(None).
        assert!(
            u.provenance("00000000-0000-4000-8000-000000000000")
                .unwrap()
                .is_none()
        );
    }

    /// #1172 Fase 1: timeline events carry actor + evidence when written via
    /// the provenance-aware API, and old callers (no provenance) stay working.
    #[test]
    fn timeline_events_carry_provenance() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("evidence chain probe", &[], None, None)
            .expect("remember");

        u.store
            .add_timeline_event_with_provenance(
                &id,
                crate::timeline::TimelineEventType::Updated,
                Some(&serde_json::json!({"field": "importance"})),
                Some("agent:cto"),
                Some(&serde_json::json!([{"memory": "other-id", "score": 0.82}])),
            )
            .expect("append provenance event");

        let events = u.timeline(&id, 0).expect("timeline");
        assert_eq!(events.len(), 2, "created + updated");
        let provenance_event = events
            .iter()
            .find(|e| e.event_type == "updated")
            .expect("updated event");
        assert_eq!(provenance_event.actor.as_deref(), Some("agent:cto"));
        let evidence = provenance_event.evidence.as_ref().expect("evidence");
        assert_eq!(evidence[0]["memory"], serde_json::json!("other-id"));

        // Plain events (Created) have no actor — backward compatible shape.
        let created = events.iter().find(|e| e.event_type == "created").unwrap();
        assert!(created.actor.is_none());
        assert!(created.evidence.is_none());
    }
}
