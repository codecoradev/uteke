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
        { text: 'Installation', link: '/install' },
        { text: 'Quick Start', link: '/getting-started' },
        { text: 'Configuration', link: '/configuration' },
        { text: 'Docker', link: '/docker' },
      ],
    },
    {
      text: 'Features',
      items: [
        { text: 'Rooms', link: '/rooms' },
        { text: 'Time-Travel', link: '/time-travel' },
        { text: 'Multi-Agent', link: '/multi-agent' },
        { text: 'Smart Decay', link: '/smart-decay' },
        { text: 'Relationship Graph', link: '/relationship-graph' },
        { text: 'Benchmarks', link: '/benchmarks' },
        { text: 'Shell Hooks', link: '/shell-hooks' },
        { text: 'MCP Server', link: '/mcp' },
      ],
    },
    {
      text: 'Reference',
      items: [
        { text: 'CLI Reference', link: '/cli-reference' },
        { text: 'Comparison', link: '/comparison' },
        { text: 'Architecture', link: '/architecture' },
        { text: 'Hermes Integration', link: '/integrations/hermes' },
        { text: 'Pi Extension', link: '/extensions' },
        { text: 'TLS & Reverse Proxy', link: '/tls' },
        { text: 'Roadmap', link: '/roadmap' },
      ],
    },
  ],
})
