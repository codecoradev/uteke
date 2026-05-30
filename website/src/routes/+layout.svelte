<script lang="ts">
	import '../app.css';

	let { children } = $props();
</script>

<svelte:head>
	<link rel="preconnect" href="https://fonts.googleapis.com" />
	<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="anonymous" />
	<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet" />
	<title>uteke — Persistent Memory for AI Agents</title>
</svelte:head>

<div class="min-h-screen flex flex-col">
	<nav class="border-b border-[var(--color-border)] bg-[var(--color-bg)]/80 backdrop-blur-md sticky top-0 z-50">
		<div class="max-w-6xl mx-auto px-6 h-14 flex items-center justify-between">
			<a href="/" class="flex items-center gap-2.5 text-base font-bold hover:text-[var(--color-accent)] transition-colors">
				<span class="text-lg">🧠</span>
				uteke
			</a>
			<div class="flex items-center gap-5 text-sm text-[var(--color-text-muted)]">
				<a href="/docs" class="hover:text-[var(--color-text)] transition-colors hidden sm:inline">Docs</a>
				<a href="/docs/roadmap" class="hover:text-[var(--color-text)] transition-colors hidden md:inline">Roadmap</a>
				<a href="https://github.com/ajianaz/uteke" target="_blank" rel="noopener" class="hover:text-[var(--color-text)] transition-colors inline-flex items-center gap-1">
					GitHub
					<svg class="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
				</a>
			</div>
		</div>
	</nav>

	<main class="flex-1">
		{@render children()}
	</main>

	<footer class="border-t border-[var(--color-border)] mt-20">
		<div class="max-w-6xl mx-auto px-6 py-8 flex flex-col sm:flex-row items-center justify-between gap-4 text-sm text-[var(--color-text-dim)]">
			<span>© 2026 uteke — local-first semantic memory for AI agents</span>
			<div class="flex items-center gap-4">
				<a href="https://github.com/ajianaz/uteke" target="_blank" rel="noopener" class="hover:text-[var(--color-text)] transition-colors">GitHub</a>
				<a href="https://github.com/ajianaz/uteke/blob/develop/LICENSE" class="hover:text-[var(--color-text)] transition-colors">Apache 2.0</a>
			</div>
		</div>
	</footer>
</div>
