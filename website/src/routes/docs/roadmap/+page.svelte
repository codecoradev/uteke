<svelte:head>
	<title>Roadmap — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Roadmap</h1>

<p class="text-[var(--color-text-muted)] mb-10">Demand-gated — we build what people actually use. Track progress on <a href="https://github.com/ajianaz/uteke/issues" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Issues</a>.</p>

<div class="space-y-10">

	<!-- Completed v0.0.2 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.2</span>
			Core Engine
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Persistent vector index (usearch)', issue: 40 },
				{ title: 'Multi-agent namespaces', issue: 39 },
				{ title: 'Tiered memory (Hot/Warm/Cold)', issue: 38 },
				{ title: 'Health checks (doctor, verify, repair)', issue: 37 },
				{ title: 'Website (landing + docs)', issue: 53 },
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.3 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.3</span>
			Tag Management & Operations
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Tag-based filtering and management', issue: 42 },
				{ title: 'Memory aging with auto-cleanup', issue: 44 },
				{ title: 'Shell hook for auto-context loading', issue: 47 },
				{ title: 'Configuration file (uteke.toml)', issue: 48 },
				{ title: 'Graceful shutdown (SIGINT handler)', issue: 31 },
				{ title: 'File logging with daily rotation', issue: 32 },
				{ title: 'Upgrade Node.js 20→24 in CI', issue: 41 },
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.4 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.4</span>
			Server Mode & Intelligence
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Bulk memory operations (tag, cold, all)', issue: 43 },
				{ title: 'Namespace switching & defaults', issue: 45 },
				{ title: 'Daemon/server mode — warm recall <50ms', issue: 54 },
				{ title: 'CLI auto-routes to server (21ms recall)', issue: 54 },
				{ title: 'Auto-forget & temporal facts', issue: 51 },
				{ title: 'Consolidation & deduplication', issue: 52 },
				{ title: 'Cora AI code review in CI', issue: 86 },
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.5 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.5</span>
			Docker & Deployment
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'UTEKE_HOME env var for custom data directory', issue: 95 },
				{ title: 'uteke-serve reads uteke.toml, default host 0.0.0.0', issue: 96 },
				{ title: 'Dockerfile (multi-stage build, model baked in)', issue: 97 },
				{ title: 'Docker image build & push on release (GHCR)', issue: 99 },
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.6 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.6</span>
			Hardening & Fixes
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'JSON output omits embedding vector (~3KB savings per memory)' },
				{ title: 'import() now persists vector index (fixes data loss on restart)' },
				{ title: 'Docker: non-root container with USER uteke' },
				{ title: 'Dependabot: automated dependency updates' },
			] as item}
				<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
					<span class="text-sm">{item.title}</span>
					<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
				</div>
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.7 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.7</span>
			Core Stability
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Tag queries: LIKE → json_each() for exact matching', issue: 120 },
				{ title: 'Configurable tier thresholds (hot_days, warm_days)', issue: 127 },
				{ title: 'Test coverage: 34 → 94 tests', issue: 129 },
				{ title: 'Tag substring false positive fix ("rust" ≠ "rustacean")' },
			] as item}
				{#if item.issue}
					<a
						href="https://github.com/ajianaz/uteke/issues/{item.issue}"
						target="_blank"
						rel="noopener"
						class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
					>
						<div class="flex items-center gap-3">
							<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
							<span class="text-sm">{item.title}</span>
						</div>
						<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
					</a>
				{:else}
					<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
						<span class="text-sm">{item.title}</span>
						<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
					</div>
				{/if}
			{/each}
		</div>
	</section>

	<!-- Completed v0.0.8 (current) -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.8</span>
			Stability & Architecture
			<span class="text-xs px-2 py-0.5 rounded-full bg-emerald-500/20 text-emerald-400 ml-2">Current</span>
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Architecture: module split (1471→352 lib.rs)', issue: 130 },
				{ title: 'Input validation (max 10K chars, 20 tags, 1MB payload)', issue: 132 },
				{ title: 'Binary checksums (SHA256) + verify-checksums subcommand', issue: 134 },
				{ title: 'Schema versioning + migration framework', issue: 138 },
				{ title: 'Error handling rewrite — sanitized user-friendly messages', issue: 144 },
				{ title: 'Python wrapper: 7→21 methods with type hints', issue: 137 },
				{ title: 'Memory benchmark binary (library-level timing)', issue: 49 },
				{ title: 'Memory consolidation command' },
				{ title: 'Import/Export (JSONL backup and restore)' },
				{ title: 'README overhaul + landing page refresh (GTM Phase 1+2)', issue: 174 },
			] as item}
				{#if item.issue}
					<a
						href="https://github.com/ajianaz/uteke/issues/{item.issue}"
						target="_blank"
						rel="noopener"
						class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
					>
						<div class="flex items-center gap-3">
							<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
							<span class="text-sm">{item.title}</span>
						</div>
						<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
					</a>
				{:else}
					<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
						<span class="text-sm">{item.title}</span>
						<span class="text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400">✓ Done</span>
					</div>
				{/if}
			{/each}
		</div>
	</section>

	<!-- Phase A — Next -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-amber-500/20 text-amber-400">Phase A</span>
			Growth (100+ stars)
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Better embeddings, richer integrations, broader reach.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'Better embeddings (larger model option)' },
				{ title: 'Import from external knowledge sources', issue: 46 },
				{ title: 'Python SDK (PyO3 bindings)' },
				{ title: 'Editor integrations (VS Code, JetBrains)' },
				{ title: 'Node.js SDK (NAPI)' },
			] as item}
				{#if item.issue}
					<a
						href="https://github.com/ajianaz/uteke/issues/{item.issue}"
						target="_blank"
						rel="noopener"
						class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
					>
						<div class="flex items-center gap-3">
							<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
							<span class="text-sm">{item.title}</span>
						</div>
						<span class="text-xs px-2 py-0.5 rounded-full bg-amber-500/20 text-amber-400">Planned</span>
					</a>
				{:else}
					<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
						<span class="text-sm">{item.title}</span>
						<span class="text-xs px-2 py-0.5 rounded-full bg-amber-500/20 text-amber-400">Planned</span>
					</div>
				{/if}
			{/each}
		</div>
	</section>

	<!-- Phase B — Future -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-blue-500/20 text-blue-400">Phase B</span>
			Collaboration (500+ stars)
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Optional cloud sync, team features, and gateway integrations.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'Cloud sync (opt-in, end-to-end encrypted)' },
				{ title: 'Team collaboration & shared namespaces' },
				{ title: 'API gateway integration (Hermes, LangChain, CrewAI)' },
				{ title: 'TLS support & reverse proxy documentation', issue: 100 },
				{ title: 'API key authentication for uteke-serve', issue: 101 },
			] as item}
				{#if item.issue}
					<a
						href="https://github.com/ajianaz/uteke/issues/{item.issue}"
						target="_blank"
						rel="noopener"
						class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
					>
						<div class="flex items-center gap-3">
							<span class="text-xs text-[var(--color-text-dim)] font-mono">#{item.issue}</span>
							<span class="text-sm">{item.title}</span>
						</div>
						<span class="text-xs px-2 py-0.5 rounded-full bg-blue-500/20 text-blue-400">Future</span>
					</a>
				{:else}
					<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
						<span class="text-sm">{item.title}</span>
						<span class="text-xs px-2 py-0.5 rounded-full bg-blue-500/20 text-blue-400">Future</span>
					</div>
				{/if}
			{/each}
		</div>
	</section>

	<!-- Phase C — Vision -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-purple-500/20 text-purple-400">Phase C</span>
			Ecosystem (1000+ stars)
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Plugin ecosystem, advanced consolidation, community extensions.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'Plugin ecosystem (custom embedding, storage backends)' },
				{ title: 'Advanced consolidation (cross-namespace, summarization)' },
				{ title: 'Community extensions marketplace' },
				{ title: 'Managed cloud API (optional paid tier)' },
			] as item}
				<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
					<span class="text-sm">{item.title}</span>
					<span class="text-xs px-2 py-0.5 rounded-full bg-purple-500/20 text-purple-400">Vision</span>
				</div>
			{/each}
		</div>
	</section>

</div>
