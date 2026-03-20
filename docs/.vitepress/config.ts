import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'LibreFang Docs',
  description: 'LibreFang Agent Operating System Documentation',
  lang: 'zh-CN',
  themeConfig: {
    nav: [
      { text: '首页', link: '/' },
      { text: 'GitHub', link: 'https://github.com/librefang/librefang' }
    ],
    sidebar: [
      {
        text: '入门',
        items: [
          { text: '快速开始', link: '/librefang' },
          { text: '发布路线图', link: '/roadmap' },
          { text: '使用示例', link: '/examples' },
          { text: '术语表', link: '/glossary' },
        ]
      },
      {
        text: '配置',
        items: [
          { text: '配置文件', link: '/configuration' },
          { text: 'LLM 提供商', link: '/providers' },
        ]
      },
      {
        text: '架构',
        items: [
          { text: '系统架构', link: '/architecture' },
          { text: '安全', link: '/security' },
        ]
      },
      {
        text: 'Agent',
        items: [
          { text: 'Agent 模板', link: '/agents' },
          { text: '自主 Hands', link: '/hands' },
          { text: '内存系统', link: '/memory' },
          { text: '技能开发', link: '/skills' },
          { text: '工作流', link: '/workflows' },
        ]
      },
      {
        text: '集成',
        items: [
          { text: '通道适配器', link: '/channels' },
          { text: 'API 参考', link: '/api' },
          { text: 'SDK 参考', link: '/sdk' },
          { text: 'CLI 参考', link: '/cli' },
          { text: 'MCP/A2A', link: '/mcp-a2a' },
          { text: '迁移指南', link: '/migration' },
          { text: '桌面应用', link: '/desktop' },
          { text: '开发指南', link: '/development' },
        ]
      },
      {
        text: '运维',
        items: [
          { text: '故障排除', link: '/troubleshooting' },
          { text: '生产部署', link: '/production' },
          { text: '常见问题', link: '/faq' },
        ]
      }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/librefang/librefang' }
    ],
    search: { provider: 'local' },
    outline: false,
  }
})
