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

[store]
# Store location (default: ~/.uteke)
path = "~/.uteke"

# Default namespace (default: "default")
namespace = "default"

[log]
# Log level: trace, debug, info, warn, error
level = "info"

# Log directory (default: ~/.uteke/logs)
dir = "~/.uteke/logs"

[server]
# Enable CLI auto-routing to server
enabled = false

# Server host
host = "127.0.0.1"

# Server port
port = 8767</code></pre>
	</section>

	<!-- Server Mode -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Server Mode</h2>
		<p class="mb-3">When <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">[server] enabled = true</code>, the CLI automatically routes commands through the running HTTP server:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Start server
uteke-serve --port 8767

# CLI commands now route via HTTP (21ms vs 980ms cold start)
uteke recall "what was that context?"
uteke remember "New finding" --tags research
uteke stats</code></pre>
		<p class="mt-3 text-sm">If the server is not running, CLI falls back to local store automatically.</p>

		<div class="mt-4 overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Setting</th><th class="text-left px-4 py-2 font-medium">Default</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">enabled</td><td class="px-4 py-2">false</td><td class="px-4 py-2">Enable CLI→server routing</td></tr>
					<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">host</td><td class="px-4 py-2">127.0.0.1</td><td class="px-4 py-2">Server bind address</td></tr>
					<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">port</td><td class="px-4 py-2">8767</td><td class="px-4 py-2">Server port</td></tr>
				</tbody>
			</table>
		</div>
	</section>

	<!-- Migration -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Config Migration</h2>
		<p class="mb-3">If you have an older flat-format config (pre-v0.0.4), uteke auto-migrates it on first run:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Old format (auto-detected and migrated)
path = "~/.uteke"
default_namespace = "default"
log_level = "info"

↓ Auto-migrated to ↓

[store]
path = "~/.uteke"
namespace = "default"

[log]
level = "info"</code></pre>
		<p class="mt-3 text-sm">No manual action needed — old config keys are automatically converted to the new sectioned format.</p>
	</section>

	<!-- Namespace Resolution -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Namespace Resolution</h2>
		<p class="mb-3">Namespace is resolved in this order (highest priority first):</p>
		<div class="space-y-2">
			{#each [
				{ n: '1', path: '--namespace flag', desc: 'CLI flag (highest priority)' },
				{ n: '2', path: 'UTEKE_NAMESPACE', desc: 'Environment variable' },
				{ n: '3', path: 'uteke.toml [store] namespace', desc: 'Config file' },
				{ n: '4', path: '"default"', desc: 'Built-in default' },
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
		<p class="mt-3 text-sm">Switch default namespace permanently with <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">uteke namespace switch &lt;name&gt;</code> — this updates the config file.</p>
	</section>

	<!-- Examples -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Per-Project Config</h2>
		<p class="mb-3">Place a <code class="px-1.5 py-0.5 rounded bg-[var(--color-surface)] border border-[var(--color-border)] text-xs">uteke.toml</code> in your project root to override defaults for that project:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># my-project/uteke.toml
[store]
path = "./.uteke"
namespace = "my-project"

[log]
level = "warn"

[server]
enabled = true
port = 8767</code></pre>
		<p class="mt-3 text-sm">Combined with shell hooks, this enables automatic project-scoped memory — each project gets its own isolated memory store.</p>
	</section>

	<!-- CLI Flag Override -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">CLI Flag Override</h2>
		<p class="mb-3">CLI flags always take precedence over config file values:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code># Override store path
uteke --store /path/to/project/.uteke remember "project note"

# Override config file
uteke --config ./my-config.toml stats

# Override namespace
uteke --namespace agent-1 recall "context"

# Override namespace via env
UTEKE_NAMESPACE=agent-1 uteke recall "context"</code></pre>
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
