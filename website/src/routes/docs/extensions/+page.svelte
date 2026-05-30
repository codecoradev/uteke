<script lang="ts">
	const installSteps = [
		{
			title: "Global install (all projects)",
			cmd: "cp -r extensions/uteke-status ~/.pi/agent/extensions/",
		},
		{
			title: "Project-local install",
			cmd: "cp -r extensions/uteke-status .pi/extensions/",
		},
		{
			title: "Reload pi",
			cmd: "/reload",
		},
	];
</script>

<svelte:head>
	<title>Pi Extension — Uteke Docs</title>
</svelte:head>

<h1 class="text-3xl font-bold mb-6">Pi Extension</h1>

<p class="text-[var(--color-text-muted)] mb-8">uteke provides a <a href="https://github.com/ajianaz/uteke" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">pi coding agent</a> extension that shows memory stats in the footer and reminds the agent to use uteke actively.</p>

<div class="space-y-10 text-[var(--color-text-muted)] leading-relaxed">

	<!-- Status Bar -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Status Bar</h2>
		<p class="mb-3">Shows memory stats across all namespaces in the pi footer:</p>
		<pre class="px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto"><code>🧠 uteke:   🔥 4 hot   🟡 0 warm   ❄️ 63 cold   (67 total)</code></pre>
		<p class="mt-3 text-sm">Auto-refreshes on session start and after each agent turn.</p>
	</section>

	<!-- Agent Memory Injection -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Agent Memory Injection</h2>
		<p class="mb-3">The extension injects a system prompt that reminds the agent to use uteke:</p>
		<div class="space-y-2">
			{#each [
				"Save important context proactively (decisions, progress, architecture)",
				"Recall relevant memories before starting tasks",
				"Use namespaces and tags for organization",
				"Save session summaries when ending",
			] as rule}
				<div class="flex items-start gap-2">
					<span class="text-[var(--color-accent)]">→</span>
					<span class="text-sm">{rule}</span>
				</div>
			{/each}
		</div>
	</section>

	<!-- Install -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Install</h2>
		<div class="space-y-4">
			{#each installSteps as step, i}
				<div class="flex gap-4">
					<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-bold text-[var(--color-accent)]">{i + 1}</div>
					<div class="min-w-0 flex-1">
						<p class="font-medium mb-2">{step.title}</p>
						<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono overflow-x-auto">{step.cmd}</code>
					</div>
				</div>
			{/each}
		</div>
	</section>

	<!-- Commands -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Commands</h2>
		<div class="overflow-x-auto rounded-lg border border-[var(--color-border)]">
			<table class="w-full text-sm">
				<thead><tr class="border-b border-[var(--color-border)] bg-[var(--color-surface)]"><th class="text-left px-4 py-2 font-medium">Command</th><th class="text-left px-4 py-2 font-medium">Description</th></tr></thead>
				<tbody>
					<tr class="border-b border-[var(--color-border)]"><td class="px-4 py-2 font-mono text-xs text-[var(--color-accent)]">/uteke-stats</td><td class="px-4 py-2">Manually refresh memory stats in status bar</td></tr>
				</tbody>
			</table>
		</div>
	</section>

	<!-- Requirements -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">Requirements</h2>
		<div class="space-y-2">
			{#each [
				"uteke installed and in PATH",
				"sqlite3 CLI (pre-installed on macOS and most Linux)",
				"pi coding agent (v0.14+)",
			] as req}
				<div class="flex items-center gap-2">
					<span class="text-[var(--color-success)]">✓</span>
					<span class="text-sm">{req}</span>
				</div>
			{/each}
		</div>
	</section>

	<!-- How it works -->
	<section>
		<h2 class="text-xl font-semibold text-[var(--color-text)] mb-4">How It Works</h2>
		<div class="space-y-3">
			{#each [
				{ label: "Stats query", desc: "Reads ~/.uteke/uteke.db via sqlite3 CLI for cross-namespace totals" },
				{ label: "System prompt", desc: "Injects uteke usage rules via before_agent_start event" },
				{ label: "Status refresh", desc: "Updates footer on session_start and turn_end events" },
			] as item}
				<div class="px-4 py-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
					<p class="text-sm font-medium text-[var(--color-text)]">{item.label}</p>
					<p class="text-xs text-[var(--color-text-dim)] mt-1">{item.desc}</p>
				</div>
			{/each}
		</div>
	</section>
</div>
