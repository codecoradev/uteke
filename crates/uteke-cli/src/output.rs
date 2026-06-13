//! Human-readable and JSON output helpers.

/// Print a value as JSON to stdout.
pub(crate) fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string(value).unwrap());
}

/// Print tags in human-readable format.
pub(crate) fn print_tags_human(tags: &[uteke_core::TagInfo], _by_count: bool) {
    if tags.is_empty() {
        println!("No tags found.");
        return;
    }
    println!("Tags ({} total):\n", tags.len());
    for t in tags {
        println!("  {} ({})", t.name, t.count);
    }
}

/// Print a confirmation after storing a memory.
pub(crate) fn print_remember_human(id: &str) {
    println!("✓ Memory stored");
    println!("  ID: {id}");
}

/// Print recall search results in human-readable format.
pub(crate) fn print_recall_human(results: &[uteke_core::SearchResult]) {
    if results.is_empty() {
        println!("No matching memories found.");
        return;
    }
    println!("Found {} result(s):\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let tags = if r.memory.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.memory.tags.join(", "))
        };
        println!(
            "  {}. (score: {:.3}) {}{}",
            i + 1,
            r.score,
            r.memory.content,
            tags
        );
        println!("     ID: {}", r.memory.id);
        println!("     Created: {}", r.memory.created_at.to_rfc3339());
    }
}

/// Print keyword search results in human-readable format.
pub(crate) fn print_search_human(results: &[uteke_core::SearchResult]) {
    if results.is_empty() {
        println!("No matching memories found.");
        return;
    }
    println!("Found {} result(s):\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let tags = if r.memory.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.memory.tags.join(", "))
        };
        println!("  {}. {}{}", i + 1, r.memory.content, tags);
        println!("     ID: {}", r.memory.id);
    }
}

/// Print a list of memories in human-readable format.
pub(crate) fn print_list_human(memories: &[uteke_core::Memory]) {
    if memories.is_empty() {
        println!("No memories found.");
        return;
    }
    for m in memories {
        let tags = if m.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", m.tags.join(", "))
        };
        println!("  {}{}", m.content, tags);
        println!("    ID: {}", m.id);
        println!("    Created: {}", m.created_at.to_rfc3339());
    }
}

/// Print a single memory in human-readable format.
pub(crate) fn print_get_human(memory: &uteke_core::Memory) {
    println!("ID: {}", memory.id);
    println!("Content: {}", memory.content);
    if !memory.tags.is_empty() {
        println!("Tags: {}", memory.tags.join(", "));
    }
    if !memory.metadata.is_null() {
        println!(
            "Metadata: {}",
            serde_json::to_string_pretty(&memory.metadata).unwrap()
        );
    }
    println!("Created: {}", memory.created_at.to_rfc3339());
    println!("Updated: {}", memory.updated_at.to_rfc3339());
}

/// Print store statistics in human-readable format.
pub(crate) fn print_stats_human(stats: &uteke_core::StoreStats) {
    println!("Memory Store Statistics");
    println!("──────────────────────");
    println!("  Total memories: {}", stats.total_memories);
    println!("  🔥 Hot (7d):    {}", stats.hot);
    println!("  🟡 Warm (30d):  {}", stats.warm);
    println!("  ❄️  Cold (>30d):  {}", stats.cold);
    println!("  Unique tags:    {}", stats.unique_tags);
    let size_str = if stats.db_size_bytes < 1024 {
        format!("{} B", stats.db_size_bytes)
    } else if stats.db_size_bytes < 1024 * 1024 {
        format!("{:.1} KB", stats.db_size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", stats.db_size_bytes as f64 / (1024.0 * 1024.0))
    };
    println!("  Database size:  {}", size_str);

    // Recall cache metrics
    let total_queries = stats.cache_hits + stats.cache_misses;
    if total_queries > 0 {
        let hit_rate = (stats.cache_hits as f64 / total_queries as f64) * 100.0;
        println!("  Cache hits:     {} ({:.0}%)", stats.cache_hits, hit_rate);
        println!("  Cache misses:   {}", stats.cache_misses);
    }
}

/// Print doctor health check report in human-readable format.
pub(crate) fn print_doctor_human(report: &uteke_core::DoctorReport) {
    println!("Uteke Health Check");
    println!("───────────────────");
    let all_ok = report
        .checks
        .iter()
        .all(|c| matches!(c.status, uteke_core::DoctorStatus::Ok));
    for check in &report.checks {
        let icon = match check.status {
            uteke_core::DoctorStatus::Ok => "✓",
            uteke_core::DoctorStatus::Warn => "⚠",
            uteke_core::DoctorStatus::Error => "✗",
        };
        println!("  {} {}: {}", icon, check.name, check.detail);
    }
    if all_ok {
        println!("\n  All checks passed.");
    } else {
        println!("\n  Some checks failed. Run `uteke repair` if index is inconsistent.");
    }
}

/// Print verify report in human-readable format.
pub(crate) fn print_verify_human(report: &uteke_core::VerifyReport) {
    println!("Verify Report");
    println!("─────────────");
    println!("  SQLite DB:    {} memories", report.db_count);
    println!("  usearch index: {} vectors", report.index_count);
    if report.consistent {
        println!("  ✓ Consistent");
    } else {
        println!("  ✗ MISMATCH — run `uteke repair` to rebuild index");
    }
}

/// Print repair report in human-readable format.
pub(crate) fn print_repair_human(report: &uteke_core::RepairReport) {
    println!("Repair Report");
    println!("─────────────");
    println!("  SQLite DB:     {} memories", report.db_count);
    println!("  Index before:  {} vectors", report.index_before);
    println!("  Index after:   {} vectors", report.index_after);
    if report.index_after == report.db_count {
        println!("  ✓ Index rebuilt successfully");
    } else {
        println!("  ⚠ Index count still differs from DB");
    }
}

/// Print aging status in human-readable format.
pub(crate) fn print_aging_status_human(status: &uteke_core::AgingStatus) {
    println!("Memory Aging Status");
    println!("────────────────────");
    println!("  Total:          {}", status.total);
    println!("  🔥 Hot (7d):    {}", status.hot);
    println!("  🟡 Warm (30d):  {}", status.warm);
    println!("  ❄️  Cold (>30d):  {}", status.cold);
    println!("  🚫 Never accessed: {}", status.never_accessed);
}

/// Print room semantic recall results in human-readable format.
pub(crate) fn print_room_semantic_human(room_id: &str, results: &[uteke_core::SearchResult]) {
    if results.is_empty() {
        println!("No matching memories found in room {room_id}.");
        return;
    }
    println!("Found {} result(s) in room {}:\n", results.len(), room_id);
    for (i, r) in results.iter().enumerate() {
        let tags = if r.memory.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.memory.tags.join(", "))
        };
        println!(
            "  {}. (score: {:.2}) {}{}",
            i + 1,
            r.score,
            r.memory.content,
            tags
        );
        println!("     ID: {}", &r.memory.id[..8.min(r.memory.id.len())]);
    }
}

/// Print aging preview (memories eligible for cleanup) in human-readable format.
pub(crate) fn print_aging_preview_human(memories: &[uteke_core::Memory]) {
    if memories.is_empty() {
        println!("No aged memories eligible for cleanup.");
        return;
    }
    println!("Aged Memories ({} eligible for cleanup):\n", memories.len());
    for (i, m) in memories.iter().enumerate() {
        let accessed = m
            .last_accessed
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        println!(
            "  {}. {}",
            i + 1,
            m.content.chars().take(80).collect::<String>()
        );
        println!("     ID: {}", m.id);
        println!("     Created: {}", m.created_at.to_rfc3339());
        println!("     Accessed: {} (count: {})", accessed, m.access_count);
    }
}

/// Print a room document in human-readable format.
pub(crate) fn print_room_document_human(doc: &uteke_core::RoomDocument) {
    // Header
    let title = doc.title.as_deref().unwrap_or(&doc.room_id);
    println!("# {}", title);
    println!("Generated: {}\n", doc.generated_at);

    for section in &doc.sections {
        println!("## {} {}", section.icon, section.heading);
        for entry in &section.entries {
            // Truncate content to 200 chars for human output
            let content = if entry.content.len() > 200 {
                format!("{}...", &entry.content[..197])
            } else {
                entry.content.clone()
            };
            let tags = if entry.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", entry.tags.join(", "))
            };
            let author = if entry.author.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.author)
            };
            println!("• {}{}{}", content, author, tags);
            println!("  {}", entry.created_at);
        }
        println!();
    }
}
