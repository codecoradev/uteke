import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Uteke',
  description: 'Local-first semantic memory engine. Single binary, zero infrastructure, 30ms recall.',
  lang: 'en',
  base: '/docs/uteke/',
  cleanUrls: true,
  head: [
    ['link', { rel: 'icon', href: '/docs/uteke/favicon.svg' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Uteke — Give Your AI a Memory' }],
    ['meta', { property: 'og:description', content: 'Offline-first semantic memory. Single binary. Zero config. 30ms recall.' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],
  srcExclude: ['**/launch/**', '**/plans/**'],
  themeConfig: {
    logo: '/docs/uteke/favicon.svg',
    nav: [
      { text: 'Codecora', link: 'https://codecora.dev' },
      { text: 'Docs', link: '/docs/uteke/getting-started' },
      { text: 'CLI Reference', link: '/docs/uteke/cli-reference' },
      { text: 'Configuration', link: '/docs/uteke/configuration' },
      { text: 'GitHub', link: 'https://github.com/codecoradev/uteke' },
    ],
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/docs/uteke/getting-started' },
          { text: 'CLI Reference', link: '/docs/uteke/cli-reference' },
          { text: 'Configuration', link: '/docs/uteke/configuration' },
          { text: 'Docker', link: '/docs/uteke/docker' },
        ],
      },
      {
        text: 'Features',
        items: [
          { text: 'Rooms', link: '/docs/uteke/getting-started#rooms' },
          { text: 'Time-Travel', link: '/docs/uteke/getting-started#time-travel-queries' },
          { text: 'Multi-Agent', link: '/docs/uteke/multi-agent' },
          { text: 'Smart Decay', link: '/docs/uteke/getting-started#memory-importance-pinning' },
          { text: 'Relationship Graph', link: '/docs/uteke/getting-started#relationship-graph' },
          { text: 'Benchmarks', link: '/docs/uteke/getting-started#benchmarking' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Architecture', link: '/docs/uteke/architecture' },
          { text: 'Pi Extension', link: '/docs/uteke/extensions' },
          { text: 'TLS & Reverse Proxy', link: '/docs/uteke/tls' },
          { text: 'Roadmap', link: '/docs/uteke/roadmap' },
        ],
      },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/codecoradev/uteke' }],
    search: { provider: 'local' },
  },
})
