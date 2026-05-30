<svelte:head>
	<title>Multi-Agent — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Multi-Agent Isolation</h1>

<p class="text-[var(--color-text-muted)] mb-8">Uteke provides first-class namespace support for running multiple AI agents, each with fully isolated memory.</p>

<div class="space-y-10 text-[var(--color-text-muted)] leading-relaxed">

	<!-- Concept -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">How Namespaces Work</h2>
		<p class="mb-3">Every memory belongs to exactly one namespace. Namespaces are fully isolated — a recall in one namespace never returns results from another.</p>
		<div class="grid grid-cols-1 md:grid-cols-3 gap-4 mt-4">
			{#each [
				{ name: 'default', desc: 'Used when no --namespace flag is provided. Backward compatible with v0.0.1 databases.' },
				{ name: 'hermes', desc: 'Example: a planning agent that remembers architecture decisions.' },
				{ name: 'pi-agent', desc: 'Example: a coding agent that remembers project-specific context.' },
			] as ns}
				<div class="px-4 py-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
					<code class="text-sm text-[var(--color-accent)]">{ns.name}</code>
					<p class="text-xs text-[var(--color-text-dim)] mt-1">{ns.desc}</p>
				</div>
			{/each}
		</div>
	</section>

	<!-- Usage -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Usage</h2>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Agent "architect" stores its context
uteke --namespace architect remember "We chose PostgreSQL for ACID compliance" --tags db,decision

# Agent "dev" has its own separate memory
uteke --namespace dev remember "Database connection string: postgres://localhost:5432/app" --tags db,config

# Each only sees its own memories
uteke --namespace architect recall "database"
# → Finds "We chose PostgreSQL for ACID compliance"

uteke --namespace dev recall "database"
# → Finds "Database connection string: postgres://localhost:5432/app"

# Without --namespace, uses "default"
uteke remember "General knowledge" --tags misc</code></pre>
	</section>

	<!-- Auto-migration -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Auto-Migration</h2>
		<p class="mb-3">Existing databases from v0.0.1 are automatically migrated on first run:</p>
		<div class="space-y-2">
			{#each [
				'namespace column added to SQLite',
				'All existing memories assigned to "default" namespace',
				'Zero data loss — your memories are preserved',
			] as item}
				<div class="flex items-center gap-2">
					<span class="text-[var(--color-success)]">✓</span>
					<span class="text-sm">{item}</span>
				</div>
			{/each}
		</div>
	</section>

	<!-- All commands scoped -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">All Commands Are Scoped</h2>
		<p class="mb-3">The <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">--namespace</code> flag works on every command:</p>
		<div class="overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Command</th><th class="text-left px-4 py-2 font-medium">Scoped Behavior</th></tr></thead>
				<tbody>
					{#each [
						{c:'remember',d:'Store in namespace'},
						{c:'recall',d:'Search within namespace'},
						{c:'search',d:'Text search within namespace'},
						{c:'list',d:'List memories in namespace'},
						{c:'tags list',d:'Tags for namespace'},
						{c:'tags rename',d:'Rename tag in namespace'},
						{c:'tags delete',d:'Delete tag in namespace'},
						{c:'aging status',d:'Aging breakdown for namespace'},
						{c:'aging cleanup',d:'Cleanup in namespace'},
						{c:'stats',d:'Statistics for namespace'},
						{c:'export',d:'Export from namespace'},
						{c:'import',d:'Import to namespace'},
					] as r}
						<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">{r.c}</td><td class="px-4 py-2">{r.d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- Best Practices -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Best Practices</h2>
		<div class="space-y-4">
			{#each [
				{ title: 'One namespace per agent role', desc: 'Use descriptive names like "planner", "coder", "reviewer" instead of "agent-1", "agent-2".' },
				{ title: 'Use config files for defaults', desc: 'Set default_namespace in uteke.toml per project so agents don\'t need --namespace on every call.' },
				{ title: 'Shell hooks for project isolation', desc: 'Install shell hooks (uteke hook install) to auto-discover .uteke/ in parent directories — each project gets isolated memory.' },
				{ title: 'Export for backup', desc: 'uteke export --namespace my-agent > backup.jsonl — backup per-agent memory independently.' },
			] as bp}
				<div class="px-4 py-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
					<p class="text-sm font-medium text-[var(--color-text)]">{bp.title}</p>
					<p class="text-xs text-[var(--color-text-dim)] mt-1">{bp.desc}</p>
				</div>
			{/each}
		</div>
	</section>
</div>
