<svelte:head>
	<title>Configuration — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Configuration</h1>

<p class="text-[var(--color-text-muted)] mb-8">Uteke supports <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">uteke.toml</code> configuration with layered resolution.</p>

<div class="space-y-10 text-[var(--color-text-muted)] leading-relaxed">

	<!-- Resolution Order -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Resolution Order</h2>
		<p class="mb-4">Uteke searches for config in this order. First match wins:</p>
		<div class="space-y-2">
			{#each [
				{ n: '1', path: './uteke.toml', desc: 'Current directory' },
				{ n: '2', path: '../uteke.toml', desc: 'Parent directories (walks up to root)' },
				{ n: '3', path: '~/.config/uteke/uteke.toml', desc: 'User-level config' },
				{ n: '4', path: '(built-in defaults)', desc: 'Hardcoded defaults' },
			] as item}
				<div class="flex items-start gap-3 px-4 py-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
					<span class="flex-shrink-0 w-6 h-6 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-xs font-bold text-[var(--color-accent)]">{item.n}</span>
					<div>
						<code class="text-sm text-[var(--color-accent)]">{item.path}</code>
						<p class="text-xs text-[var(--color-text-dim)] mt-0.5">{item.desc}</p>
					</div>
				</div>
			{/each}
		</div>
		<p class="mt-3 text-sm">Override the config file path with the <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">--config</code> flag.</p>
	</section>

	<!-- Config File -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Config File Format</h2>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># uteke.toml

# Store location (default: ~/.uteke)
store_path = "~/.uteke"

# Log level: trace, debug, info, warn, error
log_level = "info"

# Log directory (default: ~/.uteke/logs)
log_dir = "~/.uteke/logs"

# Default namespace (default: "default")
default_namespace = "default"</code></pre>
	</section>

	<!-- Examples -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Per-Project Config</h2>
		<p class="mb-3">Place a <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">uteke.toml</code> in your project root to override defaults for that project:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># my-project/uteke.toml
store_path = "./.uteke"
default_namespace = "my-project"
log_level = "warn"</code></pre>
		<p class="mt-3 text-sm">Combined with shell hooks, this enables automatic project-scoped memory — each project gets its own isolated memory store.</p>
	</section>

	<!-- Environment Variables -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">CLI Flag Override</h2>
		<p class="mb-3">CLI flags always take precedence over config file values:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Override store path
uteke --store /path/to/project/.uteke remember "project note"

# Override config file
uteke --config ./my-config.toml stats

# Override namespace
uteke --namespace agent-1 recall "context"</code></pre>
	</section>

	<!-- File Logging -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">File Logging</h2>
		<p class="mb-3">Logs are written to <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">~/.uteke/logs/uteke.log</code> with daily rotation:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>~/.uteke/logs/
├── uteke.log              # Current log
├── uteke.log.2026-05-29   # Yesterday's log
└── uteke.log.2026-05-28   # Two days ago</code></pre>
		<p class="mt-3 text-sm">Non-blocking async writer — logging never blocks memory operations. Rotated files are kept until manually deleted.</p>
	</section>
</div>
