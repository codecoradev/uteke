<script lang="ts">
	interface Feature {
		icon: string;
		title: string;
		desc: string;
	}

	interface Comparison {
		feature: string;
		uteke: string;
		mem0: string;
		letta: string;
		cognee: string;
	}

	const features: Feature[] = [
		{
			icon: '📦',
			title: 'Single Binary',
			desc: 'Zero dependencies. Copy, run, done. No Docker, no database server, no cloud API.'
		},
		{
			icon: '🔌',
			title: 'Offline First',
			desc: 'Local ONNX embeddings. No internet, no API keys. Your data never leaves your machine.'
		},
		{
			icon: '🔍',
			title: 'Semantic Recall',
			desc: 'Remember anything, recall by meaning. Not keyword match — true semantic search.'
		},
		{
			icon: '🏷️',
			title: 'Namespaces + Tags',
			desc: 'Multi-agent isolation built-in. Each agent gets its own namespace with tag-based filtering.'
		},
		{
			icon: '🛡️',
			title: 'Self-Healing',
			desc: 'Built-in doctor, verify, and repair. Index corruption? Fixed in one command.'
		},
		{
			icon: '📄',
			title: 'Portable',
			desc: 'Export to JSONL, import anywhere. Single ~/.uteke/ folder — copy it, share it, back it up.'
		}
	];

	const comparisons: Comparison[] = [
		{ feature: 'Install', uteke: '1 binary', mem0: 'pip + Docker + Qdrant', letta: 'pip + Docker + Postgres', cognee: 'pip + Docker + Neo4j/pgvector' },
		{ feature: 'Offline', uteke: '✅ Fully', mem0: '❌ Needs cloud embedding', letta: '❌ Needs server', cognee: '❌ Needs LLM + vector DB' },
		{ feature: 'Semantic Search', uteke: '✅ Local ONNX', mem0: '✅ Cloud embedding', letta: '⚠️ Keyword + archival', cognee: '✅ GraphRAG' },
		{ feature: 'Namespace', uteke: '✅ Built-in', mem0: '✅ user_id scoped', letta: '⚠️ Per-agent', cognee: '⚠️ Per-dataset' },
		{ feature: 'Speed (warm)', uteke: '~23ms*', mem0: '~50ms', letta: '~100ms', cognee: '~200ms' },
		{ feature: 'Knowledge Graph', uteke: '🔮 Planned', mem0: '❌', letta: '❌', cognee: '✅ GraphRAG' },
		{ feature: 'Auto-Forget', uteke: '🔮 Planned', mem0: '✅', letta: '✅ Core memory', cognee: '✅ TTL-based' },
		{ feature: 'License', uteke: 'MIT', mem0: 'MIT', letta: 'Apache-2.0', cognee: 'Apache-2.0' }
	];

	const audiences: Feature[] = [
		{
			icon: '🤖',
			title: 'AI Agent Developers',
			desc: 'Building Claude, GPT, or custom agents that need persistent memory across sessions. Give your agent a long-term memory — without a database server.'
		},
		{
			icon: '⌨️',
			title: 'CLI Tool Enthusiasts',
			desc: 'Love fast, zero-dependency tools that just work? uteke is a single binary you pipe into your existing workflow — shell scripts, Makefiles, CI pipelines.'
		},
		{
			icon: '🔒',
			title: 'Privacy-Focused Devs',
			desc: 'Your AI\'s memories stay on your machine. No API calls, no telemetry, no cloud. Fully offline embeddings via local ONNX runtime.'
		}
	];
</script>

<svelte:head>
	<title>uteke — Give Your AI a Memory</title>
	<meta name="description" content="uteke — Local-first semantic memory engine for AI agents. Single binary, zero infrastructure, fully offline. Give your AI a memory that stays on your machine." />
</svelte:head>

<!-- Hero -->
<section class="max-w-6xl mx-auto px-6 pt-20 md:pt-28 pb-16 text-center">
	<div class="inline-flex items-center gap-2 px-3 py-1 rounded-full border border-[var(--color-border)] text-xs text-[var(--color-text-muted)] mb-8">
		<span class="w-2 h-2 rounded-full bg-green-500"></span>
		v0.0.2 — 5 platforms, open source
	</div>

	<h1 class="text-4xl sm:text-5xl md:text-7xl font-bold tracking-tight mb-4">
		Give Your AI<br />
		<span class="hero-accent">a Memory</span>
	</h1>

	<p class="hero-tagline text-sm md:text-base font-medium text-[var(--color-accent)] mb-6 tracking-wide">
		Local-first semantic memory engine — single binary, zero infrastructure, fully offline
	</p>

	<p class="text-base md:text-xl text-[var(--color-text-muted)] max-w-2xl mx-auto mb-10 leading-relaxed">
		Your AI remembers everything — without sending data to the cloud. No server, no database, no API keys.
		Works alongside your existing tools, not instead of them.
	</p>

	<!-- Install -->
	<div class="inline-flex items-center gap-3 px-4 sm:px-6 py-3 rounded-xl bg-[var(--color-surface)] border border-[var(--color-border)] font-mono text-sm">
		<span class="text-[var(--color-text-muted)]">$</span>
		<span class="hidden sm:inline">curl -fsSL https://uteke.dev/install.sh | sh</span>
		<span class="sm:hidden">curl -fsSL uteke.dev/install.sh | sh</span>
		<button
			class="text-[var(--color-text-muted)] hover:text-[var(--color-text)] transition-colors ml-2 sm:ml-4"
			onclick={() => navigator.clipboard.writeText('curl -fsSL https://uteke.dev/install.sh | sh')}
			title="Copy"
		>
			<svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
				<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
				<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
			</svg>
		</button>
	</div>

	<p class="text-sm text-[var(--color-text-muted)] mt-4">
		Or download from
		<a href="https://github.com/ajianaz/uteke/releases" target="_blank" rel="noopener" class="text-[var(--color-accent)] hover:underline">GitHub Releases</a>
		— Linux, macOS, Windows
	</p>
</section>

<!-- Architecture Diagram -->
<section class="max-w-6xl mx-auto px-6 py-12 md:py-16">
	<h2 class="text-xl md:text-2xl font-semibold text-center mb-10">How uteke fits in</h2>
	<div class="max-w-lg mx-auto flex flex-col items-center gap-0">
		<!-- Agent box -->
		<div class="w-full max-w-xs px-6 py-4 rounded-xl bg-[var(--color-surface)] border border-[var(--color-border)] text-center">
			<div class="text-sm font-semibold">AI Agent</div>
			<div class="text-xs text-[var(--color-text-muted)] mt-1">Claude, GPT, custom agents</div>
		</div>

		<!-- Arrow 1 -->
		<div class="flex flex-col items-center py-2">
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
			<div class="px-3 py-1 rounded text-xs font-mono text-[var(--color-accent)] bg-[var(--color-surface)] border border-[var(--color-accent-dim)]">
				remember / recall
			</div>
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
		</div>

		<!-- uteke box (highlighted) -->
		<div class="uteke-box w-full max-w-xs px-6 py-4 rounded-xl text-center">
			<div class="text-sm font-bold text-[var(--color-accent)]">uteke</div>
			<div class="text-xs text-[var(--color-text-muted)] mt-1">local semantic memory</div>
		</div>

		<!-- Arrow 2 -->
		<div class="flex flex-col items-center py-2">
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
			<div class="px-3 py-1 rounded text-xs font-mono text-[var(--color-accent)] bg-[var(--color-surface)] border border-[var(--color-accent-dim)]">
				SQLite + ONNX
			</div>
			<div class="w-px h-4 bg-[var(--color-accent-dim)]"></div>
		</div>

		<!-- Storage box -->
		<div class="w-full max-w-xs px-6 py-4 rounded-xl bg-[var(--color-surface)] border border-[var(--color-border)] text-center">
			<div class="text-sm font-semibold font-mono">~/.uteke/</div>
			<div class="text-xs text-[var(--color-text-muted)] mt-1">single folder, fully portable</div>
		</div>
	</div>
</section>

<!-- Who is this for? -->
<section class="max-w-6xl mx-auto px-6 py-12 md:py-16">
	<h2 class="text-xl md:text-2xl font-semibold text-center mb-12">Who is this for?</h2>
	<div class="grid grid-cols-1 md:grid-cols-3 gap-6">
		{#each audiences as a}
			<div class="audience-card p-6 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)]">
				<span class="text-2xl">{a.icon}</span>
				<h3 class="text-lg font-medium mt-3 mb-2">{a.title}</h3>
				<p class="text-sm text-[var(--color-text-muted)] leading-relaxed">{a.desc}</p>
			</div>
		{/each}
	</div>
</section>

<!-- Features -->
<section class="max-w-6xl mx-auto px-6 py-12 md:py-16">
	<h2 class="text-xl md:text-2xl font-semibold text-center mb-12">Why uteke?</h2>
	<div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
		{#each features as feat}
			<div class="feature-card p-6 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)]">
				<span class="text-2xl">{feat.icon}</span>
				<h3 class="text-lg font-medium mt-3 mb-2">{feat.title}</h3>
				<p class="text-sm text-[var(--color-text-muted)] leading-relaxed">{feat.desc}</p>
			</div>
		{/each}
	</div>
</section>

<!-- Quick Start -->
<section class="max-w-6xl mx-auto px-6 py-12 md:py-16">
	<h2 class="text-xl md:text-2xl font-semibold text-center mb-12">Quick Start</h2>
	<div class="max-w-2xl mx-auto space-y-6">
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-semibold text-[var(--color-accent)]">1</div>
			<div class="min-w-0">
				<p class="font-medium mb-2">Remember something</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono break-all sm:break-normal whitespace-normal sm:whitespace-nowrap">uteke remember --tags project "Bond uses Go chi router for REST API"</code>
			</div>
		</div>
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-semibold text-[var(--color-accent)]">2</div>
			<div class="min-w-0">
				<p class="font-medium mb-2">Recall by meaning</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono break-all sm:break-normal whitespace-normal sm:whitespace-nowrap">uteke recall "What framework does Bond use for API?"</code>
			</div>
		</div>
		<div class="flex gap-4">
			<div class="flex-shrink-0 w-8 h-8 rounded-full bg-[var(--color-accent-dim)] flex items-center justify-center text-sm font-semibold text-[var(--color-accent)]">3</div>
			<div class="min-w-0">
				<p class="font-medium mb-2">That's it. No setup. No server. No API key.</p>
				<code class="block px-4 py-3 rounded-lg bg-[var(--color-surface)] border border-[var(--color-border)] text-sm font-mono">uteke stats</code>
			</div>
		</div>
	</div>
</section>

<!-- Comparison -->
<section class="max-w-6xl mx-auto px-6 py-12 md:py-16">
	<h2 class="text-xl md:text-2xl font-semibold text-center mb-12">How it compares</h2>
	<div class="overflow-x-auto rounded-xl border border-[var(--color-border)]">
		<table class="w-full text-sm">
			<thead>
				<tr class="border-b border-[var(--color-border)]">
					<th class="text-left px-4 py-3 font-medium text-[var(--color-text-muted)]">Feature</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-accent)]">uteke</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-muted)] hidden sm:table-cell">mem0</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-muted)] hidden md:table-cell">Letta</th>
					<th class="text-center px-4 py-3 font-medium text-[var(--color-text-muted)] hidden lg:table-cell">Cognee</th>
				</tr>
			</thead>
			<tbody>
				{#each comparisons as row}
					<tr class="border-b border-[var(--color-border)] hover:bg-[var(--color-surface)] transition-colors">
						<td class="px-4 py-3 text-[var(--color-text-muted)]">{row.feature}</td>
						<td class="px-4 py-3 text-center">{row.uteke}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-muted)] hidden sm:table-cell">{row.mem0}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-muted)] hidden md:table-cell">{row.letta}</td>
						<td class="px-4 py-3 text-center text-[var(--color-text-muted)] hidden lg:table-cell">{row.cognee}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	<p class="text-xs text-[var(--color-text-muted)] mt-4 text-center">
		* Warm speed requires server mode (planned). CLI cold start ~2.6s due to ONNX model load.
	</p>
</section>

<!-- CTA -->
<section class="max-w-6xl mx-auto px-6 py-16 md:py-20 text-center">
	<h2 class="text-2xl md:text-3xl font-bold mb-4">Ready to give your AI a brain?</h2>
	<p class="text-[var(--color-text-muted)] mb-8">Open source, MIT licensed. Start in 30 seconds.</p>
	<a
		href="https://github.com/ajianaz/uteke"
		target="_blank"
		rel="noopener"
		class="cta-button inline-flex items-center gap-2 px-6 py-3 rounded-lg bg-[var(--color-accent)] text-black font-medium"
	>
		<svg class="w-5 h-5" viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
		Star on GitHub
	</a>
</section>
