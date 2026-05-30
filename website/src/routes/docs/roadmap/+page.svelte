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

	<!-- Next -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-amber-500/20 text-amber-400">Next</span>
			Operations & Performance
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Bulk memory operations', issue: 43, status: 'open' },
				{ title: 'Smooth namespace switching and defaults', issue: 45, status: 'open' },
				{ title: 'Benchmark and performance metrics', issue: 49, status: 'open' },
				{ title: 'Daemon/server mode for warm recall (<50ms)', issue: 54, status: 'open' },
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

	<!-- Future -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-blue-500/20 text-blue-400">Future</span>
			Intelligence & Integrations
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Import from external knowledge sources', issue: 46, status: 'open' },
				{ title: 'Adopt MCP Memory Service knowledge graph', issue: 52, status: 'open' },
				{ title: 'Adopt Supermemory auto-forget & temporal facts', issue: 51, status: 'open' },
				{ title: 'Hermes plugin for uteke integration', issue: 55, status: 'open' },
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
</div>
