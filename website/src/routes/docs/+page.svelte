<svelte:head>
	<title>Getting Started — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Getting Started</h1>

<div class="space-y-8 text-[var(--color-text-muted)] leading-relaxed">
	<h2 id="install" class="text-xl font-semibold text-[var(--color-text)] !mt-0">Install</h2>

	<p>Download from <a href="https://github.com/ajianaz/uteke/releases" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Releases</a> or use the install script:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>curl -fsSL https://uteke.dev/install.sh | sh</code></pre>

	<p>Available for Linux (x64/ARM64), macOS (Intel/ARM64), and Windows (x64).</p>

	<h2 id="first-memory" class="text-xl font-semibold text-[var(--color-text)]">Your First Memory</h2>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Store a memory
uteke remember --tags project "My app uses SvelteKit 5 with Tailwind"

# Recall by meaning (semantic search)
uteke recall "What frontend framework do I use?"

# Text search
uteke search "SvelteKit"

# List all memories
uteke list

# Check system health
uteke doctor</code></pre>

	<h2 id="multi-agent" class="text-xl font-semibold text-[var(--color-text)]">Multi-Agent Isolation</h2>

	<p>Each agent gets its own namespace. Memories never leak between agents:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Agent "architect" stores its context
uteke --namespace architect remember "We chose PostgreSQL for ACID compliance"

# Agent "dev" has its own separate memory
uteke --namespace dev remember "Database connection string: postgres://localhost:5432/app"

# Each only sees its own memories
uteke --namespace architect recall "database"
uteke --namespace dev recall "database"</code></pre>

	<h2 id="export-import" class="text-xl font-semibold text-[var(--color-text)]">Export & Import</h2>

	<p>Port your memories anywhere:</p>

	<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Export to JSONL (no embeddings — small, portable)
uteke export --output memories.jsonl

# Import on another machine
uteke import --input memories.jsonl</code></pre>

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
├── uteke.db          # SQLite (memories + metadata)
├── embeddings/       # Vector index
├── model/            # Local ONNX embedding model
└── uteke.log         # Rotated logs</code></pre>

	<p>Copy the entire folder to back up or transfer to another machine.</p>
</div>
