//! Auto-generates `docs/api-reference.md` from the route registry and type schemas.
//!
//! Usage: `cargo run -p docgen`
//!
//! This binary depends on `uteke-server` with the `docgen` feature enabled,
//! giving it access to the route registry and JSON schemas for all request types.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use schemars::schema_for;
use serde_json::Value;

use uteke_server::api_registry::ENDPOINTS;

fn main() {
    let out = Path::new("docs/api-reference.md");
    let content = generate();
    fs::create_dir_all(out.parent().unwrap()).ok();
    fs::write(out, &content).expect("Failed to write api-reference.md");
    println!("✅ Generated {} ({} bytes)", out.display(), content.len());

    let core_path = Path::new("docs/core-contract.json");
    let core = core_contract();
    fs::write(core_path, &core).expect("Failed to write core-contract.json");
    println!(
        "✅ Generated {} ({} bytes)",
        core_path.display(),
        core.len()
    );
}

/// Generates `docs/core-contract.json` — the machine-readable CORE endpoint
/// list consumed by uteke-cloud's route_parity gate (uteke-cloud #81).
/// CORE/LAB semantics: see the contract doc tree `kontrak-core-cloud` in the
/// uteke PROD store. CORE changes require a uteke-cloud contract issue FIRST
/// plus an explicit owner order recorded in the contract doc.
fn core_contract() -> String {
    use uteke_server::api_registry::Tier;

    let mut core = Vec::new();
    let mut lab = Vec::new();
    for ep in ENDPOINTS {
        let entry = serde_json::json!({ "method": ep.method, "path": ep.path });
        match ep.tier {
            Tier::Core => core.push(entry),
            Tier::Lab => lab.push(entry),
        }
    }
    let doc = serde_json::json!({
        "contract": "CORE/LAB compatibility contract between uteke OSS and uteke-cloud",
        "source_of_truth": "uteke PROD store, document tree kontrak-core-cloud",
        "change_control": "CORE list changes ONLY via an explicit owner order recorded in the contract doc; OSS-side CORE semantic changes require a uteke-cloud contract issue first",
        "generated_by": "cargo run -p docgen — from uteke-server api_registry::ENDPOINTS tier tags; do not edit manually",
        "version": uteke_server::api_registry::REGISTRY_VERSION,
        "core": core,
        "lab": lab,
    });
    let mut s = serde_json::to_string_pretty(&doc).expect("serialize core contract");
    s.push('\n');
    s
}

fn generate() -> String {
    let mut md = String::new();

    // Header
    md.push_str("# HTTP API Reference\n\n");
    md.push_str(
        "Auto-generated from `uteke-server` route registry and type schemas. \
                 Do not edit manually — run `cargo run -p docgen` to regenerate.\n\n",
    );
    md.push_str("**Base URL**: `http://localhost:8767` (default)\n\n");
    md.push_str("**Auth**: Set `--auth-token <TOKEN>` to require `Authorization: Bearer <TOKEN>` header.\n\n");

    // Group endpoints by category
    let mut groups: BTreeMap<&str, Vec<&uteke_server::api_registry::Endpoint>> = BTreeMap::new();
    for ep in ENDPOINTS {
        let category = categorize(ep.path);
        groups.entry(category).or_default().push(ep);
    }

    for (category, endpoints) in &groups {
        md.push_str(&format!("## {}\n\n", category));
        for ep in endpoints {
            md.push_str(&render_endpoint(ep));
        }
        md.push('\n');
    }

    // Schemas section
    md.push_str("## Request/Response Schemas\n\n");
    md.push_str("Detailed field definitions for each request type.\n\n");

    let mut schema_types: BTreeMap<&str, Value> = BTreeMap::new();
    collect_schemas(&mut schema_types);

    for (name, schema) in &schema_types {
        md.push_str(&format!("### `{}`\n\n", name));
        md.push_str(&render_schema(schema));
        md.push('\n');
    }

    md
}

fn categorize(path: &str) -> &'static str {
    if path == "/health" {
        return "🔴 Health & Info";
    }
    if path.starts_with("/room/") || path.starts_with("/doc/room/") {
        return "🟢 Rooms";
    }
    if path.starts_with("/doc/") {
        return "🔵 Documents";
    }
    if path.starts_with("/memory/") {
        return "🟣 Memory Management";
    }
    if path == "/remember"
        || path == "/recall"
        || path == "/search"
        || path == "/list"
        || path == "/forget"
        || path == "/recent"
    {
        return "🟡 Core Memory";
    }
    if path.starts_with("/graph/") || path.starts_with("/edges") || path == "/timeline" {
        return "🟠 Graph";
    }
    if path.starts_with("/tag") {
        return "🏷️ Tags";
    }
    if path == "/pin" || path == "/unpin" {
        return "📌 Pin (Legacy)";
    }
    if path.starts_with("/import") || path.starts_with("/export") {
        return "📦 Import/Export";
    }
    if path.starts_with("/prune")
        || path.starts_with("/consolidate")
        || path.starts_with("/aging")
        || path.starts_with("/importance")
        || path.starts_with("/orphans")
        || path.starts_with("/extract")
        || path.starts_with("/rebuild")
    {
        return "🔧 Maintenance";
    }
    if path == "/context" || path == "/dream" || path == "/mcp" {
        return "🤖 AI Integration";
    }
    "📝 Other"
}

fn render_endpoint(ep: &uteke_server::api_registry::Endpoint) -> String {
    let mut s = String::new();

    // Method badge
    let badge = match ep.method {
        "GET" => "🟢 `GET`",
        "POST" => "🟡 `POST`",
        "PUT" => "🔵 `PUT`",
        "DELETE" => "🔴 `DELETE`",
        _ => ep.method,
    };

    s.push_str(&format!("#### {} `{}`\n\n", badge, ep.path));
    s.push_str(&format!("{}\n\n", ep.description));

    if ep.excludes_deprecated {
        s.push_str("*Excludes deprecated memories from results.*\n\n");
    }

    if let Some(req_type) = ep.request_type {
        if req_type == "serde_json::Value" {
            s.push_str("**Request body**: JSON object (see handler source for fields)\n\n");
        } else {
            s.push_str(&format!(
                "**Request body**: [`{}`](#{})\n\n",
                req_type,
                req_type.to_lowercase().replace('_', "-")
            ));
        }
    }

    if let Some(resp_type) = ep.response_type {
        s.push_str(&format!(
            "**Response**: [`{}`](#{})\n\n",
            resp_type,
            resp_type.to_lowercase().replace('_', "-")
        ));
    }

    if !ep.issues.is_empty() {
        let issues: Vec<String> = ep.issues.iter().map(|i| format!("`{}`", i)).collect();
        s.push_str(&format!("*Related: {}*\n\n", issues.join(", ")));
    }

    s
}

fn render_schema(schema: &Value) -> String {
    let mut s = String::new();

    if let Some(desc) = schema.get("description") {
        s.push_str(&format!("{}\n\n", desc.as_str().unwrap_or("")));
    }

    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        s.push_str("| Field | Type | Required | Description |\n");
        s.push_str("|-------|------|----------|-------------|\n");

        // Sort fields for consistent output
        let mut sorted_props: Vec<_> = props.iter().collect();
        sorted_props.sort_by_key(|(k, _)| *k);

        for (name, prop) in sorted_props {
            let ty = json_type_name(prop);
            let req = if required.contains(&name.as_str()) {
                "Yes"
            } else {
                "No"
            };
            let desc = prop
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            s.push_str(&format!("| `{}` | {} | {} | {} |\n", name, ty, req, desc));
        }
        s.push('\n');
    }

    s
}

fn json_type_name(prop: &Value) -> String {
    // Check for enum (oneOf)
    if prop.get("oneOf").is_some() {
        return "enum".to_string();
    }

    // Check for array
    if let Some(ty) = prop.get("type").and_then(|v| v.as_str()) {
        if ty == "array" {
            if let Some(items) = prop.get("items") {
                let inner = json_type_name(items);
                return format!("`{}`[]", inner);
            }
            return "array".to_string();
        }
        return format!("`{}`", ty);
    }

    // Check for $ref
    if let Some(ref_path) = prop.get("$ref").and_then(|v| v.as_str()) {
        // Extract type name from #/definitions/TypeName
        if let Some(name) = ref_path.strip_prefix("#/definitions/") {
            let anchor = name.to_lowercase().replace('_', "-");
            return format!("[`{}`](#{})", name, anchor);
        }
    }

    // Check for anyOf (Option<T>)
    if let Some(any_of) = prop.get("anyOf").and_then(|v| v.as_array()) {
        let types: Vec<String> = any_of.iter().map(json_type_name).collect();
        return types.join(" | ");
    }

    "any".to_string()
}

/// Collect JSON schemas for all named request/response types.
fn collect_schemas(schemas: &mut BTreeMap<&'static str, Value>) {
    // Note: schemars::schema_for! generates schema at compile time.
    // We use a match on type names to generate each schema individually.
    // Only types with #[derive(JsonSchema)] (via docgen feature) work here.
    macro_rules! add_schema {
        ($name:literal, $ty:ty) => {
            schemas.insert($name, serde_json::to_value(&schema_for!($ty)).unwrap());
        };
    }

    // Core types
    add_schema!("RememberRequest", uteke_server::types::RememberRequest);
    add_schema!("RecallRequest", uteke_server::types::RecallRequest);
    add_schema!("SearchRequest", uteke_server::types::SearchRequest);
    add_schema!("ListParams", uteke_server::types::ListParams);
    add_schema!(
        "MemoryUpdateRequest",
        uteke_server::types::MemoryUpdateRequest
    );
    add_schema!("MemoryPinRequest", uteke_server::types::MemoryPinRequest);
    add_schema!(
        "MemoryImportanceRequest",
        uteke_server::types::MemoryImportanceRequest
    );
    add_schema!(
        "MemoryFeedbackRequest",
        uteke_server::types::MemoryFeedbackRequest
    );

    // Room types
    add_schema!("RoomRecallRequest", uteke_server::types::RoomRecallRequest);
    add_schema!(
        "RoomRememberRequest",
        uteke_server::types::RoomRememberRequest
    );

    // Document types
    add_schema!("DocCreateRequest", uteke_server::types::DocCreateRequest);
    add_schema!("DocGetRequest", uteke_server::types::DocGetRequest);
    add_schema!("DocMoveRequest", uteke_server::types::DocMoveRequest);

    // Tags
    add_schema!("TagRenameRequest", uteke_server::types::TagRenameRequest);
    add_schema!("TagDeleteRequest", uteke_server::types::TagDeleteRequest);

    // Pin
    add_schema!("PinRequest", uteke_server::types::PinRequest);

    // Graph
    add_schema!("GraphEdgeRequest", uteke_server::types::GraphEdgeRequest);

    // Import/Export
    add_schema!("ImportRequest", uteke_server::types::ImportRequest);

    // Maintenance
    add_schema!("PruneRequest", uteke_server::types::PruneRequest);
    add_schema!(
        "ConsolidateRequest",
        uteke_server::types::ConsolidateRequest
    );
    add_schema!("AgingRequest", uteke_server::types::AgingRequest);
    add_schema!("ImportanceRequest", uteke_server::types::ImportanceRequest);
    add_schema!("OrphansRequest", uteke_server::types::OrphansRequest);
    add_schema!("ExtractRequest", uteke_server::types::ExtractRequest);

    // Response types
    add_schema!("HealthResponse", uteke_server::types::HealthResponse);
    add_schema!("ErrorResponse", uteke_server::types::ErrorResponse);
}
