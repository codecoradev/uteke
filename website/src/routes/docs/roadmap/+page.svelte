<svelte:head>
	<title>Roadmap — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Roadmap</h1>

<p class="text-[var(--color-text-muted)] mb-10">Track our progress on <a href="https://github.com/ajianaz/uteke/issues" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Issues</a>.</p>

<div class="space-y-10">
	<!-- P1 Core -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-red-500/20 text-red-400">P1</span>
			Core Memory
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Tag-based filtering and management', issue: 42, status: 'open' },
				{ title: 'Bulk memory operations', issue: 43, status: 'open' },
				{ title: 'Memory aging with auto-cleanup policy', issue: 44, status: 'open' },
				{ title: 'Smooth namespace switching and defaults', issue: 45, status: 'open' }
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors group"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-muted)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-yellow-500/20 text-yellow-400">Open</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- P2 Intelligence -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-yellow-500/20 text-yellow-400">P2</span>
			Intelligence
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Adopt Supermemory auto-forget & temporal facts', issue: 51, status: 'open' },
				{ title: 'Adopt MCP Memory knowledge graph & consolidation', issue: 52, status: 'open' },
				{ title: 'Import from external knowledge sources', issue: 46, status: 'open' }
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-muted)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-yellow-500/20 text-yellow-400">Open</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- Infrastructure -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-blue-500/20 text-blue-400">INFRA</span>
			Infrastructure
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Daemon/server mode for warm recall (<50ms)', issue: 54, status: 'open' },
				{ title: 'Hermes plugin for uteke integration (optional)', issue: 55, status: 'open' },
				{ title: 'uteke website (landing + docs + blog)', issue: 53, status: 'in-progress' }
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-muted)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full {item.status === 'in-progress'
						? 'bg-cyan-500/20 text-cyan-400'
						: 'bg-yellow-500/20 text-yellow-400'
					}">{item.status === 'in-progress' ? 'In Progress' : 'Open'}</span>
				</a>
			{/each}
		</div>
	</section>

	<!-- DX -->
	<section>
		<h2 class="text-xl font-semibold mb-4 flex items-center gap-2">
			<span class="px-2 py-0.5 rounded text-xs font-semibold bg-green-500/20 text-green-400">DX</span>
			Developer Experience
		</h2>
		<div class="space-y-3">
			{#each [
				{ title: 'Shell hook for auto-context loading', issue: 47, status: 'open' },
				{ title: 'Configuration file (uteke.toml)', issue: 48, status: 'open' },
				{ title: 'Benchmark and performance metrics', issue: 49, status: 'open' }
			] as item}
				<a
					href="https://github.com/ajianaz/uteke/issues/{item.issue}"
					target="_blank"
					rel="noopener"
					class="flex items-center justify-between px-4 py-3 rounded-lg border border-[var(--color-border)] hover:border-[var(--color-accent-dim)] transition-colors"
				>
					<div class="flex items-center gap-3">
						<span class="text-xs text-[var(--color-text-muted)] font-mono">#{item.issue}</span>
						<span class="text-sm">{item.title}</span>
					</div>
					<span class="text-xs px-2 py-0.5 rounded-full bg-yellow-500/20 text-yellow-400">Open</span>
				</a>
			{/each}
		</div>
	</section>
</div>
