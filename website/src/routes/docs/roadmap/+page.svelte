<svelte:head>
	<title>Roadmap — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Roadmap</h1>

<p class="text-[var(--color-text-muted)] mb-10">Track our progress on <a href="https://github.com/ajianaz/uteke/issues" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Issues</a>.</p>

<div class="space-y-10">

	<!-- Completed v0.0.2 -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">v0.0.2</span>
			Core Engine
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Persistent vector index (usearch)', issue: 40, status: 'done' },
				{ title: 'Multi-agent namespaces', issue: 39, status: 'done' },
				{ title: 'Tiered memory (Hot/Warm/Cold)', issue: 38, status: 'done' },
				{ title: 'Health checks (doctor, verify, repair)', issue: 37, status: 'done' },
				{ title: 'Website (landing + docs)', issue: 53, status: 'done' },
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
				{ title: 'Tag-based filtering and management', issue: 42, status: 'done' },
				{ title: 'Memory aging with auto-cleanup', issue: 44, status: 'done' },
				{ title: 'Shell hook for auto-context loading', issue: 47, status: 'done' },
				{ title: 'Configuration file (uteke.toml)', issue: 48, status: 'done' },
				{ title: 'Graceful shutdown (SIGINT handler)', issue: 31, status: 'done' },
				{ title: 'File logging with daily rotation', issue: 32, status: 'done' },
				{ title: 'Upgrade Node.js 20→24 in CI', issue: 41, status: 'done' },
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
				{ title: 'Bulk memory operations (tag, cold, all)', issue: 43, status: 'done' },
				{ title: 'Namespace switching & defaults', issue: 45, status: 'done' },
				{ title: 'Daemon/server mode — warm recall <50ms', issue: 54, status: 'done' },
				{ title: 'CLI auto-routes to server (21ms recall)', issue: 54, status: 'done' },
				{ title: 'Auto-forget & temporal facts', issue: 51, status: 'done' },
				{ title: 'Consolidation & deduplication', issue: 52, status: 'done' },
				{ title: 'Cora AI code review in CI', issue: 86, status: 'done' },
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

	<!-- v0.0.5 — In Progress -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-amber-500/20 text-amber-400">v0.0.5</span>
			Docker & Deployment
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Containerize uteke-serve with multi-stage build, flexible data paths, and automated image publishing.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'UTEKE_HOME env var for custom data directory', issue: 95 },
				{ title: 'uteke-serve reads uteke.toml, default host 0.0.0.0', issue: 96 },
				{ title: 'Dockerfile (multi-stage build, model baked in)', issue: 97 },
				{ title: 'Cora review always use latest version', issue: 98 },
				{ title: 'Docker image build & push on release', issue: 99 },
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
					<span class="text-xs px-2 py-0.5 rounded-full bg-amber-500/20 text-amber-400">Open</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- v0.0.6 — Planned -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-blue-500/20 text-blue-400">v0.0.6</span>
			Network & Auth
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Secure remote access with TLS, API key auth, and cloud routing.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'TLS support & reverse proxy documentation', issue: 100 },
				{ title: 'API key authentication for uteke-serve', issue: 101 },
				{ title: '--remote flag for cloud API routing', issue: 102 },
				{ title: 'Hermes plugin for uteke integration', issue: 55 },
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
					<span class="text-xs px-2 py-0.5 rounded-full bg-blue-500/20 text-blue-400">Planned</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- v0.0.7 — Cloud MVP -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-purple-500/20 text-purple-400">v0.0.7</span>
			Cloud MVP
		</h2>
		<p class="text-sm text-[var(--color-text-muted)] mb-4">Managed cloud API with billing and multi-tenant namespaces — open core with optional paid tier.</p>
		<div class="space-y-3">
			{#each [
				{ title: 'Managed cloud API (uteke.ajianaz.dev)' },
				{ title: 'Multi-tenant namespace isolation' },
				{ title: 'Usage-based billing (Stripe/Lynk)' },
				{ title: 'Web dashboard (usage, namespaces, settings)' },
			] as item}
				<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
					<span class="text-sm">{item.title}</span>
					<span class="text-xs px-2 py-0.5 rounded-full bg-purple-500/20 text-purple-400">Future</span>
				</div>
			{/each}
		</div>
	</section>

	<!-- Backlog -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-zinc-500/20 text-zinc-400">Backlog</span>
			Nice-to-haves
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Import from external knowledge sources', issue: 46 },
				{ title: 'Benchmark and performance metrics', issue: 49 },
				{ title: 'Better embeddings (larger model option)' },
				{ title: 'Python SDK (PyO3 bindings)' },
				{ title: 'Node.js SDK (NAPI)' },
				{ title: 'Editor integrations (VS Code, JetBrains)' },
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
						<span class="text-xs px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-400">Backlog</span>
					</a>
				{:else}
					<div class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)]">
						<span class="text-sm">{item.title}</span>
						<span class="text-xs px-2 py-0.5 rounded-full bg-zinc-500/20 text-zinc-400">Backlog</span>
					</div>
				{/if}
			{/each}
		</div>
	</section>

</div>
