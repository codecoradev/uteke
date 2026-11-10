import { createConfig } from '@codecora/theme/vitepress/config'

export default createConfig({
  product: 'uteke',
  title: 'Uteke',
  description: 'Offline semantic memory for AI agents. One binary, every MCP client, local-first. ~45ms recall.',
  accent: 'green',
  repo: 'uteke',
  head: [
    ['link', { rel: 'icon', type: 'image/png', href: '/favicon.png' }],
    ['link', { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' }],
    ['meta', { property: 'og:title', content: 'Uteke: long-term memory for any AI agent' }],
    ['meta', { property: 'og:description', content: 'Offline semantic memory for AI agents. One binary, every MCP client, local-first. ~45ms recall.' }],
    ['meta', { property: 'og:image', content: 'https://raw.githubusercontent.com/codecoradev/uteke/main/docs/assets/uteke-social-preview.png' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ['meta', { name: 'twitter:image', content: 'https://raw.githubusercontent.com/codecoradev/uteke/main/docs/assets/uteke-social-preview.png' }],
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
        { text: 'Viewer', link: '/viewer' },
        { text: 'Rooms', link: '/rooms' },
        { text: 'Time-Travel', link: '/time-travel' },
        { text: 'Multi-Agent', link: '/multi-agent' },
        { text: 'Smart Decay', link: '/smart-decay' },
        { text: 'Memory Lifecycle', link: '/memory-lifecycle' },
        { text: 'Relationship Graph', link: '/relationship-graph' },
        { text: 'Benchmarks', link: '/benchmarks' },
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
        { text: 'Claude Code Hooks', link: '/integrations/claude-code' },
        { text: 'Codex Hooks', link: '/integrations/codex' },
        { text: 'Cursor Hooks', link: '/integrations/cursor' },
        { text: 'Hermes Integration', link: '/integrations/hermes' },
        { text: 'Pi Extension', link: '/extensions' },
        { text: 'TLS & Reverse Proxy', link: '/tls' },
        { text: 'Roadmap', link: '/roadmap' },
        { text: 'Contributing', link: '/contributing/' },
      ],
    },
  ],
})
