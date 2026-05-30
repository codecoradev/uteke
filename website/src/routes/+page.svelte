<script lang="ts">
	import { onMount } from 'svelte';

	interface Feature {
		icon: string;
		title: string;
		desc: string;
		detail: string;
	}

	interface Comparison {
		feature: string;
		uteke: string;
		mem0: string;
		letta: string;
		cognee: string;
	}

	interface Command {
		cmd: string;
		desc: string;
	}

	const features: Feature[] = [
		{
			icon: '🧠',
			title: 'Semantic Memory',
			desc: 'AI remembers by meaning, not keywords',
			detail: 'Local ONNX embeddings (768d) with usearch persistent HNSW index. Recall relevant memories in ~1s, fully offline.'
		},
		{
			icon: '📦',
			title: 'Single Binary',
			desc: 'Zero dependencies. Copy, run, done.',
			detail: 'No Docker, no database server, no Python, no API keys. One Rust binary — Linux, macOS, Windows.'
		},
		{
			icon: '🏷️',
			title: 'Tags + Namespaces',
			desc: 'Multi-agent isolation built-in',
			detail: 'Each agent gets its own namespace. Tag management with list/rename/delete. Filter search by tags.'
		},
		{
			icon: '🕰️',
			title: 'Memory Aging',
			desc: 'Auto-cleanup stale memories',
			detail: 'Hot/Warm/Cold tier tracking. Preview and cleanup memories older than N days. Access-based scoring.'
		},
		{
			icon: '🐚',
			title: 'Shell Hooks',
			desc: 'Auto-load project context',
			detail: 'Install hooks for bash/zsh/fish. Auto-discovers .uteke/ in parent dirs — project-scoped memory.'
		},
		{
			icon: '🔒',
			title: 'Fully Offline',
			desc: 'Your data never leaves your machine',
			detail: 'Local ONNX embeddings. No telemetry, no cloud, no API calls. ~/.uteke/ — single folder, fully portable.'
		}
	];

	const comparisons: Comparison[] = [
		{ feature: 'Install', uteke: '1 binary', mem0: 'pip + Docker + Qdrant', letta: 'pip + Docker + Postgres', cognee: 'pip + Docker + Neo4j/pgvector' },
		{ feature: 'Offline', uteke: '✅ Fully', mem0: '❌ Needs cloud embedding', letta: '❌ Needs server', cognee: '❌ Needs LLM + vector DB' },
		{ feature: 'Semantic Search', uteke: '✅ Local ONNX', mem0: '✅ Cloud embedding', letta: '⚠️ Keyword + archival', cognee: '✅ GraphRAG' },
		{ feature: 'Tag Management', uteke: '✅ list/rename/delete', mem0: '⚠️ Basic', letta: '❌', cognee: '⚠️ Basic' },
		{ feature: 'Memory Aging', uteke: '✅ Auto-cleanup', mem0: '✅', letta: '✅ Core memory', cognee: '✅ TTL-based' },
		{ feature: 'Shell Hooks', uteke: '✅ bash/zsh/fish', mem0: '❌', letta: '❌', cognee: '❌' },
		{ feature: 'Config File', uteke: '✅ uteke.toml', mem0: '⚠️ .env', letta: '⚠️ .env', cognee: '⚠️ .env' },
		{ feature: 'License', uteke: 'Apache-2.0', mem0: 'MIT', letta: 'Apache-2.0', cognee: 'Apache-2.0' }
	];

	const commands: Command[] = [
		{ cmd: 'uteke remember "Deploy v2 to staging Friday" --tags deploy,staging', desc: 'Store a memory' },
		{ cmd: 'uteke recall "deployment schedule"', desc: 'Recall by meaning' },
		{ cmd: 'uteke tags list --by-count', desc: 'Manage tags' },
		{ cmd: 'uteke aging status', desc: 'Check memory health' },
		{ cmd: 'uteke search "monorepo" --tags rust', desc: 'Filter search by tags' },
	];

	let visibleCmd = $state(0);
	let typedText = $state('');
	let isTyping = $state(true);

	onMount(() => {
		// Cycle through commands
		const interval = setInterval(() => {
			visibleCmd = (visibleCmd + 1) % commands.length;
		}, 4000);

		return () => clearInterval(interval);
	});
</script>

<svelte:head>
	<title>uteke — Persistent Memory for AI Agents</title>
	<meta name="description" content="uteke — Local-first semantic memory engine for AI agents. Single binary, zero infrastructure, fully offline. Tags, aging, shell hooks. Give your AI a memory that stays on your machine." />
	<meta name="og:title" content="uteke — Persistent Memory for AI Agents" />
	<meta name="og:description" content="Local-first semantic memory engine. Single Rust binary, zero infrastructure, fully offline." />
	<meta name="og:type" content="website" />
</svelte:head>

<!-- Hero -->
<section class="grid-bg relative overflow-hidden">
	<!-- Subtle top gradient -->
	<div class="absolute inset-0 bg-gradient-to-b from-amber-950/10 to-transparent pointer-events-none"></div>

	<div class="relative max-w-6xl mx-auto px-6 pt-24 md:pt-36 pb-20 text-center">
		<!-- Badge -->
		<div class="animate-fade-in inline-flex items-center gap-2 px-3 py-1.5 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)] text-xs text-[var(--color-text-muted)] mb-8">
			<span class="w-2 h-2 rounded-full bg-[var(--color-success)]"></span>
			<span>v0.0.3 released</span>
			<span class="text-[var(--color-text-dim)]">—</span>
			<a href="https://github.com/ajianaz/uteke/releases/tag/v0.0.3" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">release notes →</a>
		</div>

		<!-- Headline -->
		<h1 class="animate-fade-in-delay-1 text-4xl sm:text-5xl md:text-7xl font-bold tracking-tight mb-6 leading-[1.1]">
			Your AI forgets<br />
			everything<span class="text-[var(--color-text-dim)]">.</span>
			<br />
			<span class="hero-gradient glow-text">Fix that.</span>
		</h1>

		<!-- Subheadline -->
		<p class="animate-fade-in-delay-2 text-base md:text-xl text-[var(--color-text-muted)] max-w-2xl mx-auto mb-10 leading-relaxed">
			uteke gives AI agents persistent, searchable memory.<br class="hidden md:block" />
			Fully local. Zero infrastructure. Single binary.
		</p>

		<!-- Install command -->
		<div class="animate-fade-in-delay-3 inline-flex flex-col items-center gap-4">
			<div class="terminal-block inline-flex items-center gap-3 px-5 py-3.5 rounded-xl font-mono text-sm glow-amber">
				<span class="text-[var(--color-success)]">$</span>
				<span class="hidden sm:inline text-[var(--color-text)]">cargo install --git https://github.com/ajianaz/uteke</span>
				<span class="sm:hidden text-[var(--color-text)]">cargo install --git github.com/ajianaz/uteke</span>
				<button
					class="text-[var(--color-text-dim)] hover:text-[var(--color-accent)] transition-colors ml-2"
					onclick={() => navigator.clipboard.writeText('cargo install --git https://github.com/ajianaz/uteke')}
					title="Copy"
				>
					<svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
						<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
					</svg>
				</button>
			</div>

			<div class="flex items-center gap-3 text-sm">
				<a
					href="https://github.com/ajianaz/uteke"
					target="_blank"
					rel="noopener"
					class="cta-primary inline-flex items-center gap-2 px-5 py-2.5 rounded-lg bg-[var(--color-accent)] text-black font-semibold text-sm"
				>
					<svg class="w-4 h-4" viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
					GitHub
				</a>
				<a
					href="/docs"
					class="cta-secondary inline-flex items-center gap-2 px-5 py-2.5 rounded-lg border border-[var(--color-border)] text-[var(--color-text-muted)] font-medium text-sm"
				>
					Documentation
				</a>
			</div>
		</div>
	</div>
</section>

<!-- Terminal Demo -->
<section class="max-w-6xl mx-auto px-6 -mt-4 pb-16 md:pb-24">
	<div class="terminal-block rounded-2xl overflow-hidden glow-amber">
		<!-- Title bar -->
		<div class="flex items-center gap-2 px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-surface)]">
			<div class="flex gap-1.5">
				<div class="w-3 h-3 rounded-full bg-red-500/60"></div>
				<div class="w-3 h-3 rounded-full bg-yellow-500/60"></div>
				<div class="w-3 h-3 rounded-full bg-green-500/60"></div>
			</div>
			<span class="text-xs text-[var(--color-text-dim)] ml-2 font-mono">uteke — AI memory engine</span>
		</div>

		<!-- Terminal content -->
		<div class="p-5 md:p-6 font-mono text-sm space-y-3 min-h-[220px]">
			{#each commands as c, i}
				<div class="transition-all duration-300 {i === visibleCmd ? 'opacity-100' : 'opacity-30'}">
					{#if i === visibleCmd}
						<div class="flex items-start gap-2">
							<span class="text-[var(--color-success)] select-none">❯</span>
							<div>
								<span class="cmd-highlight">{c.cmd.split(' ')[0]}</span>
								<span class="text-[var(--color-text)]"> {c.cmd.split(' ').slice(1).join(' ')}</span>
							</div>
						</div>
						<p class="text-[var(--color-text-dim)] ml-5 mt-0.5 text-xs"># {c.desc}</p>
						<div class="ml-5 mt-2 text-[var(--color-text-muted)] text-xs">
							{#if i === 0}
								<span>✓ Memory stored</span>
								<span class="text-[var(--color-text-dim)]"> ID: a1b2c3d4-...</span>
							{:else if i === 1}
								<span>1 result (0.94 similarity)</span>
								<br/><span class="text-[var(--color-text-dim)]">"Deploy v2 to staging Friday"</span>
							{:else if i === 2}
								<span class="cmd-string">deploy</span><span class="text-[var(--color-text-dim)]"> (12) </span>
								<span class="cmd-string">staging</span><span class="text-[var(--color-text-dim)]"> (5) </span>
								<span class="cmd-string">rust</span><span class="text-[var(--color-text-dim)]"> (8)</span>
							{:else if i === 3}
								<span>🔥 Hot: 42 · 🟡 Warm: 15 · ❄️ Cold: 3</span>
							{:else if i === 4}
								<span>2 results in stress namespace</span>
							{/if}
						</div>
					{:else}
						<div class="flex items-start gap-2">
							<span class="text-[var(--color-text-dim)] select-none">❯</span>
							<span class="text-[var(--color-text-dim)]">{c.cmd}</span>
						</div>
					{/if}
				</div>
			{/each}
		</div>
	</div>
</section>

<div class="separator-gradient"></div>

<!-- Problem → Solution -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-24">
	<div class="max-w-3xl mx-auto text-center">
		<p class="text-sm font-medium text-[var(--color-accent)] mb-4 tracking-wide uppercase">The Problem</p>
		<h2 class="text-2xl md:text-4xl font-bold mb-6">
			Every time your AI restarts,<br />it's a blank slate
		</h2>
		<p class="text-base md:text-lg text-[var(--color-text-muted)] leading-relaxed mb-12">
			You tell it the same context. Re-explain the same architecture. Re-describe the same preferences.
			Over and over. What if your agent could just <span class="text-[var(--color-text)]">remember</span>?
		</p>
	</div>

	<div class="max-w-3xl mx-auto">
		<div class="grid grid-cols-1 md:grid-cols-2 gap-6">
			<!-- Before -->
			<div class="terminal-block rounded-xl p-5">
				<p class="text-xs font-medium text-red-400 mb-3">❌ Without uteke</p>
				<div class="font-mono text-xs space-y-2 text-[var(--color-text-dim)]">
					<p>> What framework does Bond use?</p>
					<p class="text-red-400/70">I don't have context about "Bond"</p>
					<p>> No, I told you yesterday...</p>
					<p class="text-red-400/70">I don't have memory of previous sessions</p>
					<p>> (sighs) It uses Go chi router...</p>
				</div>
			</div>

			<!-- After -->
			<div class="terminal-block rounded-xl p-5" style="border-color: var(--color-accent-dim);">
				<p class="text-xs font-medium text-[var(--color-success)] mb-3">✅ With uteke</p>
				<div class="font-mono text-xs space-y-2">
					<p class="text-[var(--color-text-dim)]">> What framework does Bond use?</p>
					<p class="text-[var(--color-success)]">Bond uses Go chi router for REST API</p>
					<p class="text-[var(--color-text-dim)]">> How do we deploy again?</p>
					<p class="text-[var(--color-success)]">Deploy v2 to staging on Friday</p>
				</div>
			</div>
		</div>
	</div>
</section>

<div class="separator-gradient"></div>

<!-- Features -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-24">
	<div class="text-center mb-14">
		<p class="text-sm font-medium text-[var(--color-accent)] mb-4 tracking-wide uppercase">What's Inside</p>
		<h2 class="text-2xl md:text-4xl font-bold">Everything your AI needs<br />to never forget</h2>
	</div>

	<div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5">
		{#each features as feat}
			<div class="feature-card p-6 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)]">
				<span class="text-2xl">{feat.icon}</span>
				<h3 class="text-lg font-semibold mt-3 mb-1">{feat.title}</h3>
				<p class="text-sm text-[var(--color-accent)] mb-2">{feat.desc}</p>
				<p class="text-sm text-[var(--color-text-dim)] leading-relaxed">{feat.detail}</p>
			</div>
		{/each}
	</div>
</section>

<div class="separator-gradient"></div>

<!-- Quick Start -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-24">
	<div class="text-center mb-14">
		<p class="text-sm font-medium text-[var(--color-accent)] mb-4 tracking-wide uppercase">Get Started</p>
		<h2 class="text-2xl md:text-4xl font-bold">30 seconds to<br />persistent memory</h2>
	</div>

	<div class="max-w-2xl mx-auto space-y-6">
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-bold text-[var(--color-accent-bright)]">1</div>
			<div class="min-w-0 flex-1">
				<p class="font-medium mb-2">Install</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono text-[var(--color-text-dim)] overflow-x-auto">
					<span class="cmd-highlight">cargo install</span> --git https://github.com/ajianaz/uteke
				</code>
			</div>
		</div>
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-bold text-[var(--color-accent-bright)]">2</div>
			<div class="min-w-0 flex-1">
				<p class="font-medium mb-2">Store a memory</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono text-[var(--color-text-dim)] overflow-x-auto">
					<span class="cmd-highlight">uteke remember</span> <span class="cmd-string">"Deploy v2.1 to staging Friday"</span> <span class="cmd-flag">--tags</span> deploy,staging
				</code>
			</div>
		</div>
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-bold text-[var(--color-accent-bright)]">3</div>
			<div class="min-w-0 flex-1">
				<p class="font-medium mb-2">Recall by meaning</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono text-[var(--color-text-dim)] overflow-x-auto">
					<span class="cmd-highlight">uteke recall</span> <span class="cmd-string">"when do we deploy?"</span>
				</code>
			</div>
		</div>
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-bold text-[var(--color-accent)]">✓</div>
			<div class="min-w-0 flex-1">
				<p class="font-medium text-[var(--color-success)]">That's it. No setup. No server. No API key.</p>
				<p class="text-sm text-[var(--color-text-dim)] mt-1">First run downloads the embedding model (~188MB). Then it just works.</p>
			</div>
		</div>
	</div>
</section>

<div class="separator-gradient"></div>

<!-- Comparison -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-24">
	<div class="text-center mb-14">
		<p class="text-sm font-medium text-[var(--color-accent)] mb-4 tracking-wide uppercase">Compare</p>
		<h2 class="text-2xl md:text-4xl font-bold">See how it stacks up</h2>
	</div>

	<div class="overflow-x-auto rounded-xl border border-[var(--color-border)]">
		<table class="compare-table w-full text-sm">
			<thead>
				<tr class="border-b border-[var(--color-border)]">
					<th class="text-left px-4 py-3 font-medium text-[var(--color-text-dim)]">Feature</th>
					<th class="highlight-col text-center px-4 py-3 font-semibold text-[var(--color-accent)]">uteke</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-dim)] hidden sm:table-cell">mem0</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-dim)] hidden md:table-cell">Letta</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-dim)] hidden lg:table-cell">Cognee</th>
				</tr>
			</thead>
			<tbody>
				{#each comparisons as row}
					<tr class="border-b border-[var(--color-border-subtle)] hover:bg-[var(--color-surface)] transition-colors">
						<td class="px-4 py-3 text-[var(--color-text-muted)]">{row.feature}</td>
						<td class="highlight-col px-4 py-3 text-center font-medium">{row.uteke}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-dim)] hidden sm:table-cell">{row.mem0}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-dim)] hidden md:table-cell">{row.letta}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-dim)] hidden lg:table-cell">{row.cognee}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
</section>

<div class="separator-gradient"></div>

<!-- Architecture -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-24">
	<div class="text-center mb-14">
		<p class="text-sm font-medium text-[var(--color-accent)] mb-4 tracking-wide uppercase">Under the Hood</p>
		<h2 class="text-2xl md:text-4xl font-bold">Simple architecture,<br />powerful results</h2>
	</div>

	<div class="max-w-lg mx-auto flex flex-col items-center gap-0">
		<!-- Agent -->
		<div class="w-full max-w-xs px-6 py-4 rounded-xl bg-[var(--color-surface)] border border-[var(--color-border)] text-center">
			<div class="text-sm font-semibold">AI Agent</div>
			<div class="text-xs text-[var(--color-text-dim)] mt-1">Claude, GPT, custom agents, scripts</div>
		</div>

		<div class="flex flex-col items-center py-2">
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
			<div class="px-3 py-1 rounded text-xs font-mono text-[var(--color-accent)] bg-[var(--color-surface)] border border-[var(--color-accent-dim)]">
				remember / recall / search
			</div>
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
		</div>

		<!-- uteke (highlighted) -->
		<div class="w-full max-w-xs px-6 py-4 rounded-xl text-center border-2 border-[var(--color-accent)]" style="background-color: color-mix(in srgb, var(--color-accent-dim) 10%, transparent);">
			<div class="text-sm font-bold text-[var(--color-accent)]">uteke</div>
			<div class="text-xs text-[var(--color-text-muted)] mt-1">local semantic memory engine</div>
		</div>

		<div class="flex flex-col items-center py-2">
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
			<div class="px-3 py-1 rounded text-xs font-mono text-[var(--color-accent)] bg-[var(--color-surface)] border border-[var(--color-accent-dim)]">
				ONNX + usearch + SQLite
			</div>
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
		</div>

		<!-- Storage -->
		<div class="w-full max-w-xs px-6 py-4 rounded-xl bg-[var(--color-surface)] border border-[var(--color-border)] text-center">
			<div class="text-sm font-semibold font-mono">~/.uteke/</div>
			<div class="text-xs text-[var(--color-text-dim)] mt-1">single folder · fully portable · no cloud</div>
		</div>
	</div>

	<!-- Tech badges -->
	<div class="flex flex-wrap justify-center gap-3 mt-10">
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">Rust</span>
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">SQLite</span>
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">usearch HNSW</span>
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">ONNX Runtime</span>
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">EmbeddingGemma</span>
		<span class="px-3 py-1.5 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-xs font-mono text-[var(--color-text-muted)]">Apache 2.0</span>
	</div>
</section>

<div class="separator-gradient"></div>

<!-- CTA -->
<section class="max-w-6xl mx-auto px-6 py-20 md:py-28 text-center">
	<h2 class="text-2xl md:text-4xl font-bold mb-4">
		Ready to give your AI<br />
		<span class="hero-gradient">a memory?</span>
	</h2>
	<p class="text-[var(--color-text-muted)] mb-8 text-base">Open source · Apache 2.0 · Start in 30 seconds</p>
	<div class="flex items-center justify-center gap-4">
		<a
			href="https://github.com/ajianaz/uteke"
			target="_blank"
			rel="noopener"
			class="cta-primary inline-flex items-center gap-2 px-6 py-3 rounded-lg bg-[var(--color-accent)] text-black font-semibold"
		>
			<svg class="w-5 h-5" viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
			Star on GitHub
		</a>
		<a
			href="/docs"
			class="cta-secondary inline-flex items-center gap-2 px-6 py-3 rounded-lg border border-[var(--color-border)] text-[var(--color-text-muted)] font-medium"
		>
			Read the docs
		</a>
	</div>
</section>
