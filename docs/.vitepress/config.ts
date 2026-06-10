import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'uteke',
  description: 'Local-first semantic memory engine. Single binary, zero infrastructure, 30ms recall.',
  lang: 'en',
  cleanUrls: true,
  head: [
    ['link', { rel: 'icon', href: '/favicon.svg' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'uteke — Give Your AI a Memory' }],
    ['meta', { property: 'og:description', content: 'Offline-first semantic memory. Single binary. Zero config. 30ms recall.' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],
  srcExclude: ['**/launch/**', '**/plans/**'],
  themeConfig: {
    logo: '/favicon.svg',
    nav: [
      { text: 'Docs', link: '/getting-started' },
      { text: 'CLI Reference', link: '/cli-reference' },
      { text: 'Configuration', link: '/configuration' },
    ],
    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/getting-started' },
          { text: 'CLI Reference', link: '/cli-reference' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Multi-Agent Isolation', link: '/multi-agent' },
          { text: 'Pi Extension', link: '/extensions' },
          { text: 'Architecture', link: '/architecture' },
          { text: 'Roadmap', link: '/roadmap' },
        ],
      },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/codecoradev/uteke' }],
    search: { provider: 'local' },
  },
})
