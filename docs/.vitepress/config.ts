import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Uteke',
  description: 'Local-first semantic memory engine. Single binary, zero infrastructure, 30ms recall.',
  lang: 'en',
  cleanUrls: true,
  base: '/docs/uteke/',
  head: [
    ['link', { rel: 'icon', href: '/favicon.svg' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Uteke — Give Your AI a Memory' }],
    ['meta', { property: 'og:description', content: 'Offline-first semantic memory. Single binary. Zero config. 30ms recall.' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],
  srcExclude: ['**/launch/**', '**/plans/**'],
  themeConfig: {
    logo: '/favicon.svg',
    nav: [
      { text: 'Codecora', link: 'https://codecora.dev' },
      { text: 'Docs', link: '/getting-started' },
      { text: 'CLI Reference', link: '/cli-reference' },
      { text: 'Configuration', link: '/configuration' },
      { text: 'GitHub', link: 'https://github.com/codecoradev/uteke' },
    ],
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/getting-started' },
          { text: 'CLI Reference', link: '/cli-reference' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Docker', link: '/docker' },
        ],
      },
      {
        text: 'Features',
        items: [
          { text: 'Rooms', link: '/getting-started#rooms' },
          { text: 'Time-Travel', link: '/getting-started#time-travel-queries' },
          { text: 'Multi-Agent', link: '/multi-agent' },
          { text: 'Smart Decay', link: '/getting-started#memory-importance-pinning' },
          { text: 'Relationship Graph', link: '/getting-started#relationship-graph' },
          { text: 'Benchmarks', link: '/getting-started#benchmarking' },
          { text: 'MCP Server', link: '/mcp' },
          { text: 'Document Commands', link: '/cli-reference#document-commands-406-411-438-440' },
          { text: 'Graph API', link: '/cli-reference#graph-api-408-542' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Architecture', link: '/architecture' },
          { text: 'Hermes Integration', link: '/integrations/hermes' },
          { text: 'Pi Extension', link: '/extensions' },
          { text: 'TLS & Reverse Proxy', link: '/tls' },
          { text: 'Roadmap', link: '/roadmap' },
        ],
      },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/codecoradev/uteke' }],
    search: { provider: 'local' },
  },
})
