<svelte:head>
	<title>Getting Started — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Getting Started</h1>

<div class="space-y-8 text-[var(--color-text-muted)] leading-relaxed">
	<h2 id="install" class="text-xl font-semibold text-[var(--color-text)] !mt-0">Install</h2>

	<p>Install from source (requires <a href="https://rustup.rs" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">Rust</a>):</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>cargo install --git https://github.com/ajianaz/uteke</code></pre>

	<p>Or download pre-built binaries from <a href="https://github.com/ajianaz/uteke/releases" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Releases</a> — available for Linux (x64/ARM64), macOS (Apple Silicon), and Windows (x64).</p>

	<h2 id="first-memory" class="text-xl font-semibold text-[var(--color-text)]">Your First Memory</h2>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Store a memory
uteke remember --tags project "My app uses SvelteKit 5 with Tailwind"

# Recall by meaning (semantic search)
uteke recall "What frontend framework do I use?"

# Text search with tag filter
uteke search "SvelteKit" --tags project

# List all memories
uteke list

# Check system health
uteke doctor</code></pre>

	<h2 id="tags" class="text-xl font-semibold text-[var(--color-text)]">Tag Management</h2>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># List all tags with usage counts
uteke tags list --by-count

# Rename a tag across all memories
uteke tags rename old-name new-name

# Delete a tag from all memories
uteke tags delete unused-tag</code></pre>

	<h2 id="aging" class="text-xl font-semibold text-[var(--color-text)]">Memory Aging</h2>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Show hot/warm/cold/never-accessed breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --days 90

# Delete stale memories older than 180 days
uteke aging cleanup --days 180 --confirm</code></pre>

	<h2 id="multi-agent" class="text-xl font-semibold text-[var(--color-text)]">Multi-Agent Isolation</h2>

	<p>Each agent gets its own namespace. Memories never leak between agents:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Agent "architect" stores its context
uteke --namespace architect remember "We chose PostgreSQL for ACID compliance"

# Agent "dev" has its own separate memory
uteke --namespace dev remember "Database connection string: postgres://localhost:5432/app"

# Each only sees its own memories
uteke --namespace architect recall "database"
uteke --namespace dev recall "database"</code></pre>

	<h2 id="shell-hooks" class="text-xl font-semibold text-[var(--color-text)]">Shell Hooks</h2>

	<p>Auto-load project-scoped memory when you cd into a project directory:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Install hook for your shell
uteke hook install bash   # or zsh, fish

# Now when you cd into a project with .uteke/uteke.db,
# uteke auto-discovers it</code></pre>

	<h2 id="export-import" class="text-xl font-semibold text-[var(--color-text)]">Export & Import</h2>

	<p>Port your memories anywhere:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Export to JSONL (no embeddings — small, portable)
uteke export > memories.jsonl

# Import on another machine
uteke import memories.jsonl</code></pre>

	<h2 id="troubleshooting" class="text-xl font-semibold text-[var(--color-text)]">Troubleshooting</h2>

	<p>If something goes wrong, uteke has built-in self-healing:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Check system health (DB, index, model, consistency)
uteke doctor

# Verify DB and index consistency
uteke verify

# Repair index by rebuilding from SQLite
uteke repair</code></pre>

	<h2 id="storage" class="text-xl font-semibold text-[var(--color-text)]">Where is data stored?</h2>

	<p>All data lives in <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">~/.uteke/</code>:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>~/.uteke/
├── uteke.db                    # SQLite (memories + metadata)
├── uteke_index.usearch         # Persistent vector index
├── uteke_index.keys            # Index key mapping
├── models/embeddinggemma/      # Local ONNX embedding model
└── logs/
    ├── uteke.log               # Current log
    └── uteke.log.YYYY-MM-DD    # Rotated logs</code></pre>

	<p>Copy the entire folder to back up or transfer to another machine.</p>
</div>
