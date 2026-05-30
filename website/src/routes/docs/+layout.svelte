<script lang="ts">
	import { page } from '$app/stores';

	const docsNav = [
		{ label: 'Getting Started', href: '/docs' },
		{ label: 'CLI Reference', href: '/docs/cli-reference' },
		{ label: 'Configuration', href: '/docs/configuration' },
		{ label: 'Multi-Agent', href: '/docs/multi-agent' },
		{ label: 'Pi Extension', href: '/docs/extensions' },
		{ label: 'Roadmap', href: '/docs/roadmap' }
	];

	let currentPath = $derived($page.url.pathname);

	let { children } = $props();
</script>

<div class="max-w-6xl mx-auto px-6">
	<div class="flex gap-8">
		<!-- Sidebar -->
		<aside class="hidden md:block w-56 flex-shrink-0 pt-8">
			<nav class="sticky top-20 space-y-1">
				<p class="text-xs font-semibold text-[var(--color-text-dim)] uppercase tracking-wider mb-3">Documentation</p>
				{#each docsNav as item}
					<a
						href={item.href}
						class="block px-3 py-2 rounded-lg text-sm transition-colors {currentPath === item.href
							? 'bg-[var(--color-surface)] text-[var(--color-accent)] border border-[var(--color-border)]'
							: 'text-[var(--color-text-muted)] hover:text-[var(--color-text)] hover:bg-[var(--color-surface)]'
						}"
					>
						{item.label}
					</a>
				{/each}
			</nav>

			<div class="mt-8 px-3 py-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
				<p class="text-xs text-[var(--color-text-dim)] mb-1">Version</p>
				<p class="text-xs text-[var(--color-accent)] font-mono">v0.0.3</p>
			</div>
		</aside>

		<!-- Content -->
		<div class="flex-1 min-w-0 py-8">
			{@render children()}
		</div>
	</div>
</div>
