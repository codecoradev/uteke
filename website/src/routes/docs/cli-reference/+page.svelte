<script lang="ts">
	const globalFlags = [
		{ flag: '--store <path>', desc: 'Override store location', def: '~/.uteke' },
		{ flag: '--namespace <name>', desc: 'Namespace for multi-agent isolation', def: 'default' },
		{ flag: '--config <path>', desc: 'Override config file path', def: 'auto-resolved' },
		{ flag: '--json', desc: 'Output as JSON', def: 'off' },
		{ flag: '--verbose', desc: 'Enable debug logging', def: 'off' },
	];

	const rememberFlags = [
		{ f: '--tags <tags>', d: 'Comma-separated tags' },
		{ f: '--metadata <json>', d: 'Arbitrary JSON metadata' },
		{ f: '--detect-contradiction', d: 'Detect conflicting memories (threshold 0.65)' },
		{ f: '--type <type>', d: 'Memory type: fact, procedure, preference, decision, context' },
		{ f: '--valid-from <datetime>', d: 'Start of validity period (ISO 8601)' },
		{ f: '--valid-until <datetime>', d: 'End of validity period (ISO 8601)' },
		{ f: '--json', d: 'Output stored memory as JSON' },
	];

	const recallFlags = [
		{ f: '--limit <n>', d: 'Max results (default: 5)' },
		{ f: '--json', d: 'Output as JSON array' },
	];

	const searchFlags = [
		{ f: '--tags <tags>', d: 'Filter by comma-separated tags' },
		{ f: '--limit <n>', d: 'Max results (default: 20)' },
		{ f: '--json', d: 'Output as JSON' },
	];

	const listFlags = [
		{ f: '--tag <tag>', d: 'Filter by single tag' },
		{ f: '--limit <n>', d: 'Max results (default: 20)' },
		{ f: '--offset <n>', d: 'Skip first N results' },
		{ f: '--json', d: 'Output as JSON' },
	];

	const forgetFlags = [
		{ f: '--tag <tag>', d: 'Delete all memories with this tag' },
		{ f: '--cold', d: 'Delete all cold-tier memories (>30 days)' },
		{ f: '--all', d: 'Delete all memories in namespace' },
		{ f: '--confirm', d: 'Skip confirmation prompt' },
		{ f: '--dry-run', d: 'Preview what would be deleted' },
	];

	const otherCommands = [
		{ c: 'uteke get <id>', d: 'Retrieve a single memory by UUID' },
		{ c: 'uteke forget <id>', d: 'Delete a memory by UUID' },
		{ c: 'uteke forget --tag <tag>', d: 'Delete all memories with a tag' },
		{ c: 'uteke forget --cold', d: 'Delete all cold-tier memories' },
		{ c: 'uteke forget --all', d: 'Delete all memories in namespace' },
		{ c: 'uteke consolidate', d: 'Find and merge duplicate memories' },
		{ c: 'uteke prune', d: 'Remove deprecated/expired memories' },
		{ c: 'uteke stats', d: 'Show store statistics with tier breakdown' },
		{ c: 'uteke export', d: 'Export memories to JSONL (no embeddings)' },
		{ c: 'uteke import <file>', d: 'Import memories from JSONL' },
		{ c: 'uteke doctor', d: 'Health check (DB, index, model, consistency)' },
		{ c: 'uteke verify', d: 'Verify DB and index consistency' },
		{ c: 'uteke repair', d: 'Rebuild index from SQLite' },
		{ c: 'uteke namespace list', d: 'List all namespaces with memory counts' },
		{ c: 'uteke namespace stats <name>', d: 'Show stats for a namespace' },
		{ c: 'uteke namespace switch <name>', d: 'Set default namespace in config' },
		{ c: 'uteke hook install <shell>', d: 'Install shell hook (bash/zsh/fish)' },
		{ c: 'uteke completions <shell>', d: 'Generate shell completions' },
	];
</script>

<svelte:head>
	<title>CLI Reference — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">CLI Reference</h1>

<p class="text-[var(--color-text-muted)] mb-8">Complete reference for all uteke commands. Version <strong>0.0.4</strong>.</p>

<div class="space-y-10 text-[var(--color-text-muted)] leading-relaxed">

	<!-- Global Flags -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Global Flags</h2>
		<div class="overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead>
					<tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]">
						<th class="text-left px-4 py-2 font-medium">Flag</th>
						<th class="text-left px-4 py-2 font-medium">Description</th>
						<th class="text-left px-4 py-2 font-medium">Default</th>
					</tr>
				</thead>
				<tbody>
					{#each globalFlags as row}
						<tr class="border-b border-[var(--color-border)]">
							<td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{row.flag}</td>
							<td class="px-4 py-2">{row.desc}</td>
							<td class="px-4 py-2 text-[var(--color-text-dim)]">{row.def}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- remember -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke remember</h2>
		<p class="mb-3">Store a new memory with optional tags, metadata, and contradiction detection.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Basic
uteke remember "Deploy v2.1 to staging Friday" --tags deploy,staging

# With contradiction detection
uteke remember "Server runs on port 8080" --tags config --detect-contradiction

# With memory type and temporal bounds
uteke remember "API rate limit is 1000/min" --type fact --valid-from 2026-01-01 --valid-until 2026-12-31

# In a specific namespace
uteke remember "User prefers dark mode" --tags pref --namespace my-agent</code></pre>
		<div class="mt-3 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Flag</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each rememberFlags as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.f}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- recall -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke recall</h2>
		<p class="mb-3">Semantic search using vector similarity. Hot memories (accessed within 7 days) get a score boost.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>uteke recall "What framework does the API use?"
uteke recall "deployment" --limit 10
uteke recall "database config" --namespace hermes --json</code></pre>
		<div class="mt-3 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Flag</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each recallFlags as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.f}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- search -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke search</h2>
		<p class="mb-3">Keyword text search with tag filtering.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>uteke search "monorepo"
uteke search "deploy" --tags staging,prod --limit 20
uteke search "api" --namespace backend --json</code></pre>
		<div class="mt-3 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Flag</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each searchFlags as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.f}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- list -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke list</h2>
		<p class="mb-3">List memories with optional tag filter and pagination.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>uteke list --limit 20
uteke list --tag deploy --offset 10 --json
uteke list --namespace hermes</code></pre>
		<div class="mt-3 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Flag</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each listFlags as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.f}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- forget (bulk) -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke forget</h2>
		<p class="mb-3">Delete memories by ID, tag, tier, or all. Supports bulk operations.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Delete single memory
uteke forget <uuid> --confirm

# Delete all with a tag
uteke forget --tag deploy --confirm

# Delete all cold-tier memories (>30 days old)
uteke forget --cold --confirm

# Delete everything in namespace
uteke forget --all --confirm

# Preview without deleting
uteke forget --tag stale --dry-run</code></pre>
		<div class="mt-3 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Flag</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each forgetFlags as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.f}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- consolidate -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke consolidate</h2>
		<p class="mb-3">Find and merge duplicate memories using cosine similarity.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Preview duplicates (dry run)
uteke consolidate --threshold 0.60 --dry-run

# Merge duplicates
uteke consolidate --threshold 0.60

# Higher threshold = more conservative (default: 0.90)
uteke consolidate --dry-run</code></pre>
		<p class="mt-3 text-sm">Recommended threshold: <strong>0.60–0.70</strong> for small embedding models (embeddinggemma-q4). Keeps newer memory, removes older.</p>
	</section>

	<!-- prune -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke prune</h2>
		<p class="mb-3">Remove deprecated and expired temporal memories.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Preview what would be pruned
uteke prune --ttl 30 --dry-run

# Prune deprecated/expired memories
uteke prune --ttl 30</code></pre>
	</section>

	<!-- namespace -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke namespace</h2>
		<p class="mb-3">Manage namespaces — list, inspect, and switch defaults.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># List all namespaces with counts
uteke namespace list

# Show stats for a namespace
uteke namespace stats my-agent

# Switch default namespace (saved to config)
uteke namespace switch my-agent</code></pre>
		<p class="mt-3 text-sm">Namespace resolution order: <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">--namespace flag</code> → <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">UTEKE_NAMESPACE</code> env → <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">uteke.toml</code> → <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">"default"</code></p>
	</section>

	<!-- tags -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke tags</h2>
		<p class="mb-3">Manage tags across all memories.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># List all tags with counts
uteke tags list --by-count

# Rename a tag
uteke tags rename old-tag new-tag

# Delete a tag from all memories
uteke tags delete unused-tag</code></pre>
	</section>

	<!-- aging -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke aging</h2>
		<p class="mb-3">Memory aging management with auto-cleanup.</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Show hot/warm/cold breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --days 90

# Delete memories older than 180 days
uteke aging cleanup --days 180 --confirm</code></pre>
	</section>

	<!-- Other commands -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Other Commands</h2>
		<div class="overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Command</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					{#each otherCommands as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.c}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- Server Mode -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">uteke-serve (Server Mode)</h2>
		<p class="mb-3">Start a persistent HTTP server for fast AI agent access. Eliminates cold start (~980ms → ~21ms per operation).</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Start server on default port (8767)
uteke-serve

# Custom port
uteke-serve --port 9000

# With logging
RUST_LOG=info uteke-serve --port 8767</code></pre>
		<p class="mt-3 text-sm">When <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">[server] enabled = true</code> is set in config, the CLI auto-routes commands through the server. Falls back to local store if server is not running.</p>
	</section>
</div>
