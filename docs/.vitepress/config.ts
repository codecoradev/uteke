import { createConfig } from '@codecora/theme/vitepress/config'

export default createConfig({
  product: 'uteke',
  title: 'Uteke',
  description: 'Offline semantic memory for AI agents. One binary, every MCP client, local-first. ~45ms recall.',
  accent: 'green',
  repo: 'uteke',
  head: [
    ['meta', { property: 'og:title', content: 'Uteke — One Memory. Every Agent. Local-first.' }],
    ['meta', { property: 'og:description', content: 'Offline semantic memory for AI agents. One binary, every MCP client, local-first. ~45ms recall.' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],
  ignoreDeadLinks: true,
  sidebar: [
    {
      text: 'Getting Started',
      items: [
        { text: 'Installation', link: '/install' },
        { text: 'Quick Start', link: '/getting-started' },
        { text: 'Organizing Memories', link: '/organizing-memories' },
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
        { text: 'Memory Lifecycle', link: '/memory-lifecycle' },
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
        { text: 'HTTP API Reference', link: '/api-reference' },
        { text: 'Comparison', link: '/comparison' },
        { text: 'Architecture', link: '/architecture' },
        { text: 'Hermes Integration', link: '/integrations/hermes' },
        { text: 'Pi Extension', link: '/extensions' },
        { text: 'TLS & Reverse Proxy', link: '/tls' },
        { text: 'Roadmap', link: '/roadmap' },
        { text: 'Contributing', link: '/contributing/' },
      ],
    },
  ],
})
