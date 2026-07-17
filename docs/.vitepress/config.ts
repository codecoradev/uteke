import { createConfig } from '@codecora/theme/vitepress/config'

export default createConfig({
  product: 'uteke',
  title: 'Uteke',
  description: 'Local-first semantic memory engine. Single binary, zero infrastructure, 30ms recall.',
  accent: 'green',
  repo: 'uteke',
  head: [
    ['meta', { property: 'og:title', content: 'Uteke — Give Your AI a Memory' }],
    ['meta', { property: 'og:description', content: 'Offline-first semantic memory. Single binary. Zero config. 30ms recall.' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],
  ignoreDeadLinks: true,
  srcExclude: ['**/launch/**', '**/plans/**'],
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
})
