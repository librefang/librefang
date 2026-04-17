export interface Language {
  code: string
  name: string
  url: string
}

export interface PerformanceRow {
  metric: string
  others: string
  librefang: string
}

export interface Translation {
  nav: { architecture: string; hands: string; performance: string; install: string; downloads?: string; docs: string; features?: string; evolution?: string; workflows?: string; registry?: string; learnMore?: string }
  hero: {
    badge: string
    title1: string
    title2: string
    typing: string[]
    desc: string
    getStarted: string
    viewGithub: string
  }
  stats: { coldStart: string; memory: string; security?: string; channels?: string; hands?: string; providers?: string }
  architecture: {
    label: string
    title: string
    desc: string
    layers: { label: string; desc: string }[]
    kernelDescs?: string[]
    runtimeDescs?: string[]
    hardwareDescs?: string[]
  }
  hands: {
    label: string
    title: string
    desc: string
    items: { name: string; desc: string }[]
    more: string
  }
  performance: {
    label: string
    title: string
    desc: string
    metric: string
    others: string
    rows: PerformanceRow[]
  }
  install: {
    label: string
    title: string
    desc: string
    terminal: string
    comment: string
    requires: string
    includes: string
    reqItems: string[]
    incItems: string[]
  }
  faq: {
    label: string
    title: string
    items: { q: string; a: string }[]
  }
  community: {
    label: string
    title: string
    desc: string
    items: { label: string; desc: string }[]
    open: string
  }
  meta?: {
    title: string
    description: string
  }
  workflows?: {
    label: string
    title: string
    desc: string
    items: { title: string; desc: string }[]
  }
  docs?: {
    label: string
    title: string
    desc: string
    categories: { title: string; desc: string }[]
    viewAll: string
  }
  githubStats?: {
    label: string
    title: string
    desc: string
    stars: string
    forks: string
    issues: string
    prs: string
    downloads: string
    docsVisits: string
    lastUpdate: string
    starHistory: string
    starUs: string
    discuss: string
  }
  contributing?: {
    label: string
    title: string
    desc: string
    steps: { title: string; desc: string }[]
    cta: string
  }
  evolution?: {
    label: string
    title: string
    desc: string
    tagline: string
    howItWorks: { title: string; desc: string }[]
    tools: { name: string; desc: string }[]
    toolsHeading: string
    cta: string
  }
  registry?: {
    label: string
    total: string
    matching: string
    all: string
    searchPlaceholder: string
    loading: string
    errorTitle: string
    errorDesc: string
    emptyTitle: string
    emptyDesc: string
    contribute: string
    noMatches: string
    backHome: string
    sourceHint: string
    readDocs: string
    manifest: string
    copy: string
    manifestErrorTitle: string
    allIn: string
    useIt: string
    configOnly: string
    relatedIn: string
    retry: string
    openInDashboard: string
    lastUpdated: string
    copyLink: string
    trending: string
    sort?: { label: string; popular: string; nameAsc: string; nameDesc: string; trending: string }
    onThisPage: string
    previous: string
    next: string
    prevNext: string
    readme: string
    viewHistory?: string
    templateDiff?: string
    categories: {
      skills: { title: string; desc: string }
      mcp: { title: string; desc: string }
      plugins: { title: string; desc: string }
      hands: { title: string; desc: string }
      agents: { title: string; desc: string }
      providers: { title: string; desc: string }
      workflows: { title: string; desc: string }
      channels: { title: string; desc: string }
      integrations: { title: string; desc: string }
    }
  }
  search?: {
    title: string
    placeholder: string
    close: string
    noResults: string
    hint: string
    kbd: string
    open: string
  }
  browse?: {
    title: string
    desc: string
  }
  notFound?: {
    title: string
    desc: string
    home: string
  }
  pwa?: {
    title: string
    desc: string
    install: string
    dismiss: string
  }
  footer: { docs: string; license: string; privacy: string; changelog: string }
}

export const languages: Language[] = [
  { code: 'en', name: 'English', url: '/' },
  { code: 'zh', name: '简体中文', url: '/zh' },
  { code: 'zh-TW', name: '繁體中文', url: '/zh-TW' },
  { code: 'ja', name: '日本語', url: '/ja' },
  { code: 'ko', name: '한국어', url: '/ko' },
  { code: 'de', name: 'Deutsch', url: '/de' },
  { code: 'es', name: 'Español', url: '/es' },
]

export const translations: Record<string, Translation> = {
  en: {
    nav: { architecture: 'Architecture', hands: 'Hands', performance: 'Performance', install: 'Install', downloads: 'Downloads', docs: 'Docs', features: 'Marketplace', evolution: 'Skills Self-Evolution', workflows: 'Workflows', registry: 'Registry', learnMore: 'Features' },
    hero: {
      badge: 'Open Source',
      title1: 'The Agent',
      title2: 'Operating System',
      typing: [
        'run autonomous agents 24/7',
        'replace entire workflows',
        'deploy on any hardware',
        'monitor with 16 security layers',
      ],
      desc: 'LibreFang is a production-grade runtime for autonomous AI agents. Single binary, {handsCount} built-in capability units, {channelsCount} channel adapters. Built in Rust for the workloads that can\'t afford to go down.',
      getStarted: 'Get Started',
      viewGithub: 'View on GitHub',
    },
    stats: { coldStart: 'Cold Start', memory: 'Memory', security: 'Security Layers', channels: 'Channels', hands: 'Hands', providers: 'Providers' },
    architecture: {
      label: 'System Design',
      title: 'Five-layer architecture',
      desc: 'From hardware to user-facing channels. Each layer is isolated, testable, and replaceable.',
      layers: [
        { label: 'Channels', desc: '44 adapters: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
        { label: 'Hands', desc: '15 autonomous capability units with dedicated models and tools' },
        { label: 'Kernel', desc: 'Agent lifecycle, workflow orchestration, budget control, scheduling' },
        { label: 'Runtime', desc: 'Tokio async, WASM sandbox, Merkle audit chain, SSRF protection' },
        { label: 'Hardware', desc: 'Single binary: laptop, VPS, Raspberry Pi, bare metal, cloud' },
      ],
      kernelDescs: [
        'Create, start, pause, resume, stop, destroy',
        '9 built-in templates, DAG orchestration',
        'Per-agent spend limits, global caps, alerts',
        'Cron-based triggers, interval tasks, event hooks',
        'Short-term, long-term, episodic, semantic',
        'Python & prompt skills, hot-reload',
        'Model Context Protocol, Agent-to-Agent',
        'Open Fang Protocol for mesh networking',
      ],
      runtimeDescs: [
        'Multi-threaded async runtime',
        'Isolated execution for untrusted code',
        'Hash-chain integrity verification',
        'Block internal network access',
        'Data flow analysis for secrets',
        'Token bucket per agent/channel',
        'Multi-layer input sanitization',
        'Role-based access control + audit log',
      ],
      hardwareDescs: [
        '32MB, zero dependencies, just copy and run',
        'x86_64 and ARM64 native builds',
        'ARM64 — runs on Pi 4 with 64MB RAM',
        'Termux environment, ARM64 native',
        'Any $5/mo VPS, Docker optional',
        'Direct deployment, no orchestrator needed',
        'Native desktop app with system tray',
      ],
    },
    hands: {
      label: 'Capability Units',
      title: '15 built-in Hands',
      desc: 'Each Hand ships with its own model, tools, and workflow. Activate, don\'t assemble.',
      items: [
        { name: 'Clip', desc: 'YouTube to vertical shorts with AI captions. Auto-publish to Telegram.' },
        { name: 'Lead', desc: 'Daily prospect discovery with ICP scoring, dedup, and CSV export.' },
        { name: 'Collector', desc: 'OSINT-grade intelligence monitoring with change detection.' },
        { name: 'Predictor', desc: 'Calibrated probabilistic forecasting for markets and outcomes.' },
        { name: 'Researcher', desc: 'Deep research with source credibility scoring.' },
        { name: 'Trader', desc: 'Autonomous market intelligence and trading engine — multi-signal analysis, adversarial reasoning, risk management.' },
      ],
      more: '+ 9 more: Twitter, Browser, Analytics, DevOps, Creator, LinkedIn, Reddit, Strategist, API Tester',
    },
    performance: {
      label: 'Benchmarks',
      title: 'Built different',
      desc: 'Rust, not TypeScript. Production, not prototype.',
      metric: 'Metric',
      others: 'Others',
      rows: [
        { metric: 'Cold Start', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: 'Idle Memory', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: 'Binary Size', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: 'Security Layers', others: '2 ~ 3', librefang: '16' },
        { metric: 'Channel Adapters', others: '8 ~ 15', librefang: '44' },
        { metric: 'Built-in Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: 'Get Started',
      title: 'One command',
      desc: 'Single binary. No Docker. 60 seconds to autonomous agents.',
      terminal: 'terminal',
      comment: '# agents are now running autonomously',
      requires: 'Requires',
      includes: 'Includes',
      reqItems: ['Linux / macOS / Windows', '64MB RAM minimum', 'x86_64 or ARM64', 'LLM API Key'],
      incItems: ['{handsCount} built-in Hands', '{channelsCount} channel adapters', '{providersCount} LLM providers', 'Desktop app (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: 'Common questions',
      items: [
        { q: 'What is LibreFang?', a: 'A production-grade Agent Operating System built in Rust. It runs autonomous AI agents on schedules 24/7 — without requiring user prompts. Runtime, security, and channel infrastructure in a single binary.' },
        { q: 'What are Hands?', a: 'Self-contained autonomous capability units. Each Hand has a dedicated model, tools, and workflow. 15 built-in: Clip (video), Lead (prospecting), Collector (OSINT), Predictor (forecasting), Researcher, Trader, and more.' },
        { q: 'Which LLM providers are supported?', a: '50 providers: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM, and more. 200+ models total. Each Hand can use a different provider.' },
        { q: 'Which channels are supported?', a: '44 channel adapters: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC, and more.' },
        { q: 'Is it production-ready?', a: '2100+ tests, zero Clippy warnings. 16 security layers including WASM sandbox, Merkle audit chain, SSRF protection. Pin your version until v1.0 for stability.' },
      ],
    },
    community: {
      label: 'Open Source',
      title: 'Join the community',
      desc: 'LibreFang is built in the open. Contribute code, report issues, or join the discussion.',
      items: [
        { label: 'Contribute', desc: 'Submit PRs, fix bugs, improve docs' },
        { label: 'Report', desc: 'Found a bug? Open an issue' },
        { label: 'Discuss', desc: 'Ask questions, share ideas' },
        { label: 'Discord', desc: 'Join our Discord server' },
      ],
      open: 'Open',
    },
    meta: {
      title: 'LibreFang - The Agent Operating System',
      description: 'LibreFang is a production-grade Agent Operating System built in Rust. 180ms cold start, 40MB memory, 16 security layers, 44 channel adapters. Run autonomous AI agents 24/7.',
    },
    workflows: {
      label: 'Workflows',
      title: 'Replace entire workflows',
      desc: 'LibreFang doesn\'t just assist — it takes over. These are the operations you\'d otherwise hire people for.',
      items: [
        { title: 'Content Pipeline', desc: 'Clip + Twitter: monitor trending videos, cut shorts, add captions, publish to social — all while you\'re offline.' },
        { title: 'Sales Prospecting', desc: 'Lead runs nightly: discovers prospects, scores by ICP fit, removes duplicates, exports clean CSV.' },
        { title: 'Competitive Intelligence', desc: 'Collector watches competitor sites, pricing, job boards, and news. Alerts the moment something changes.' },
        { title: 'Multi-Agent Orchestration', desc: 'Chain Hands with workflow orchestration: Researcher → Predictor → Clip → broadcast to 44 channels.' },
        { title: 'Migration', desc: 'One command: librefang migrate --from openclaw. Agents, memory, and skills transfer automatically.' },
        { title: 'Production Security', desc: 'WASM sandbox, Merkle audit chain, SSRF protection, prompt injection scanning, GCRA rate limiting — 16 layers.' },
      ],
    },
    docs: {
      label: 'Documentation',
      title: 'Documentation',
      desc: 'Comprehensive guides for LibreFang',
      categories: [
        { title: 'Overview', desc: 'Introduction, quick start, architecture' },
        { title: 'Automation', desc: 'Cron tasks, webhooks, integrations' },
        { title: 'Infrastructure', desc: 'Deployment, monitoring, scaling' },
      ],
      viewAll: 'View All Docs',
    },
    githubStats: {
      label: 'Community',
      title: 'Join the community',
      desc: 'Help us build the future of autonomous AI agents',
      stars: 'Stars', forks: 'Forks', issues: 'Issues', prs: 'PRs',
      downloads: 'Downloads', docsVisits: 'Docs Visits', lastUpdate: 'Last Update',
      starHistory: 'Star History', starUs: 'Star Us', discuss: 'Discuss',
    },
    contributing: {
      label: 'Contributing',
      title: 'How to contribute',
      desc: 'LibreFang is open source and welcomes contributions of all kinds.',
      steps: [
        { title: 'Fork & Clone', desc: 'Fork the repository and clone it locally to get started.' },
        { title: 'Pick an Issue', desc: 'Browse open issues labeled "good first issue" or "help wanted".' },
        { title: 'Submit a PR', desc: 'Make your changes, write tests, and submit a pull request for review.' },
      ],
      cta: 'Read Contributing Guide',
    },
    evolution: {
      label: 'Skills Self-Evolution',
      title: 'Agents that teach themselves',
      desc: 'After a complex task, a background LLM review decides whether the approach is worth saving. New skills hot-reload into the runtime — no restart.',
      tagline: 'Autonomous. Versioned. Security-scanned.',
      toolsHeading: 'Evolution tools',
      howItWorks: [
        { title: 'Automatic detection', desc: '5+ tool calls trigger a background review of the approach.' },
        { title: 'Hot-reload', desc: 'New and updated skills are available immediately — no daemon restart.' },
        { title: 'Security scanning', desc: 'Every mutation passes through prompt injection detection with auto-rollback.' },
        { title: 'Version history', desc: 'Up to 10 versions per skill with timestamps, changelogs, and rollback snapshots.' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: 'Save a successful approach as a new prompt-only skill.' },
        { name: 'skill_evolve_update', desc: 'Rewrite a skill\'s prompt context entirely.' },
        { name: 'skill_evolve_patch', desc: 'Targeted find-and-replace with 5-strategy fuzzy matching.' },
        { name: 'skill_evolve_rollback', desc: 'Revert to the previous version instantly.' },
        { name: 'skill_evolve_write_file', desc: 'Add supporting files: references, templates, scripts, assets.' },
        { name: 'skill_evolve_delete', desc: 'Remove a locally-created skill.' },
      ],
      cta: 'Read Skill Evolution Docs',
    },
    registry: {
      label: 'Registry',
      total: 'total',
      matching: 'matching',
      all: 'All',
      searchPlaceholder: 'Search by name, id, or tag...',
      loading: 'Loading registry…',
      errorTitle: 'Could not load registry',
      errorDesc: 'GitHub rate limit hit or the proxy is down. Retry in a few seconds.',
      emptyTitle: 'Nothing here yet',
      emptyDesc: 'This section of the registry is not populated yet. Check back soon or contribute one.',
      contribute: 'Contribute on GitHub',
      noMatches: 'No matches for',
      backHome: 'Home',
      sourceHint: 'Data proxied from the librefang-registry repo on GitHub.',
      readDocs: 'Read the docs',
      manifest: 'Manifest',
      copy: 'Copy',
      manifestErrorTitle: 'Could not load manifest',
      allIn: 'All {category}',
      useIt: 'Use it',
      configOnly: '{category} entries are configured through ~/.librefang/config.toml rather than a CLI install command. Copy the manifest below and paste it into the matching section of your config.',
      relatedIn: 'More {category}',
      retry: 'Retry',
      openInDashboard: 'Or install via local dashboard',
      lastUpdated: 'Updated',
      copyLink: 'Copy link to this section',
      trending: 'Trending',
      sort: { label: 'Sort', popular: 'Popular', nameAsc: 'Name A–Z', nameDesc: 'Name Z–A', trending: 'Most clicked' },
      onThisPage: 'On this page',
      previous: 'Previous',
      next: 'Next',
      prevNext: 'Previous / next in category',
      readme: 'README',
      viewHistory: 'History',
      templateDiff: 'Template diff',
      categories: {
        skills: { title: 'Skills', desc: 'Pluggable tool bundles — Python, WASM, Node, or prompt-only skills that extend what an agent can do.' },
        mcp:    { title: 'MCP Servers', desc: 'Model Context Protocol servers that plug external tools and data sources directly into any agent.' },
        plugins:{ title: 'Plugins', desc: 'Runtime extensions that add custom commands, channels, or behaviors to the LibreFang daemon.' },
        hands:  { title: 'Hands', desc: 'Autonomous capability units. Each Hand ships with its own model, tools, and workflow — activate, don\'t assemble.' },
        agents: { title: 'Agents', desc: 'Pre-built agent templates. Model, system prompt, capabilities and scheduling — all in one manifest.' },
        providers: { title: 'Providers', desc: 'LLM provider adapters. Anthropic, OpenAI, Gemini, Groq, local — and the 40+ in between.' },
        workflows: { title: 'Workflows', desc: 'Multi-step agent orchestrations expressed as TOML. Chain agents, branch on conditions, persist state.' },
        channels:  { title: 'Channels', desc: 'Messaging adapters. Telegram, Slack, Discord, WhatsApp, LINE and 40+ other platforms.' },
        integrations: { title: 'Integrations', desc: 'First-party integrations that expose agents to common SaaS tools and protocols.' },
      },
    },
    search: {
      title: 'Search registry',
      placeholder: 'Search skills, hands, agents, providers…',
      close: 'Close',
      noResults: 'No matches for "{query}"',
      hint: 'Type to search across all registry entries.',
      kbd: '↑↓ navigate · ↵ open · esc close',
      open: 'Search',
    },
    browse: {
      title: 'Browse the registry',
      desc: 'Every category at a glance — pick one to see every entry, sorted by popularity.',
    },
    notFound: {
      title: 'Page not found',
      desc: "We couldn't find what you were looking for.",
      home: 'Back to home',
    },
    pwa: {
      title: 'Install LibreFang',
      desc: 'Add the site to your home screen or dock.',
      install: 'Install',
      dismiss: 'Dismiss',
    },
    footer: { docs: 'Docs', license: 'License', privacy: 'Privacy', changelog: 'Changelog' },
  },

  zh: {
    nav: { architecture: '架构', hands: '能力单元', performance: '性能', install: '安装', downloads: '下载', docs: '文档', features: '市场', evolution: '技能自我进化', workflows: '工作流', registry: '注册表', learnMore: '功能' },
    hero: {
      badge: '开源',
      title1: 'Agent',
      title2: '操作系统',
      typing: [
        '7x24 运行自主 Agent',
        '替代整条工作流',
        '部署到任意硬件',
        '16 层安全防护',
      ],
      desc: 'LibreFang 是面向自主 AI Agent 的生产级运行时。单一二进制文件，{handsCount} 个内置能力单元，{channelsCount} 个渠道适配器。Rust 构建，为不能停机的负载而生。',
      getStarted: '开始使用',
      viewGithub: '查看 GitHub',
    },
    stats: { coldStart: '冷启动', memory: '内存', security: '安全层', channels: '渠道', hands: '能力单元', providers: '模型提供商' },
    architecture: {
      label: '系统设计',
      title: '五层架构',
      desc: '从硬件到用户渠道，每一层都隔离、可测试、可替换。',
      layers: [
        { label: '渠道层', desc: '44 个渠道适配器：Telegram、Slack、Discord、飞书、钉钉、WhatsApp...' },
        { label: '能力层', desc: '15 个自主能力单元，各配专属模型和工具' },
        { label: '内核层', desc: 'Agent 生命周期、工作流编排、预算控制、调度' },
        { label: '运行时层', desc: 'Tokio 异步、WASM 沙箱、Merkle 审计链、SSRF 防护' },
        { label: '硬件层', desc: '单一二进制：笔记本、VPS、树莓派、裸金属、云' },
      ],
      kernelDescs: ['创建、启动、暂停、恢复、停止、销毁', '9 个内置模板，DAG 编排', '单 Agent 限额、全局上限、告警', '基于 Cron 的触发器、定时任务、事件钩子', '短期、长期、情景、语义', 'Python 和 Prompt 技能，热重载', 'Model Context Protocol，Agent 间通信', 'Open Fang Protocol，网状网络'],
      runtimeDescs: ['多线程异步运行时', '不可信代码隔离执行', '哈希链完整性验证', '阻止内部网络访问', '敏感数据流分析', '每 Agent/渠道令牌桶', '多层输入清洗', '基于角色的访问控制 + 审计日志'],
      hardwareDescs: ['32MB，零依赖，复制即运行', 'x86_64 和 ARM64 原生构建', 'ARM64 — Pi 4 上 64MB 内存可运行', 'Termux 环境，ARM64 原生', '任意 $5/月 VPS，Docker 可选', '直接部署，无需编排器', '原生桌面应用，系统托盘'],
    },
    hands: {
      label: '能力单元',
      title: '15 个内置 Hand',
      desc: '每个 Hand 自带模型、工具和工作流。激活即用，无需组装。',
      items: [
        { name: 'Clip', desc: 'YouTube 视频自动转竖版短视频，AI 字幕，自动发布到 Telegram。' },
        { name: 'Lead', desc: '每日自主发现潜在客户，ICP 评分、去重、CSV 导出。' },
        { name: 'Collector', desc: 'OSINT 级情报监控，变化检测。' },
        { name: 'Predictor', desc: '校准概率预测引擎，预判市场和业务走向。' },
        { name: 'Researcher', desc: '深度自主研究，来源可信度评分。' },
        { name: 'Trader', desc: '自主市场情报与交易引擎——多信号分析、对抗推理、风险管理。' },
      ],
      more: '+ 另有 9 个 Hand：Twitter、Browser、Analytics、DevOps、Creator、LinkedIn、Reddit、Strategist、API Tester',
    },
    performance: {
      label: '基准测试',
      title: '生而不同',
      desc: 'Rust 而非 TypeScript。生产级，非原型。',
      metric: '指标',
      others: '其他框架',
      rows: [
        { metric: '冷启动', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: '空闲内存', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: '二进制体积', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: '安全层', others: '2 ~ 3', librefang: '16' },
        { metric: '渠道适配器', others: '8 ~ 15', librefang: '44' },
        { metric: '内置 Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: '开始使用',
      title: '一条命令',
      desc: '单一二进制文件，无需 Docker，60 秒启动自主 Agent。',
      terminal: '终端',
      comment: '# Agent 已在自主运行',
      requires: '系统要求',
      includes: '包含内容',
      reqItems: ['Linux / macOS / Windows', '最低 64MB 内存', 'x86_64 或 ARM64', 'LLM API 密钥'],
      incItems: ['{handsCount} 个内置 Hands', '{channelsCount} 个渠道适配器', '{providersCount} 个 LLM 提供商', '桌面应用 (Tauri 2.0)'],
    },
    faq: {
      label: '常见问题',
      title: '常见问题',
      items: [
        { q: '什么是 LibreFang？', a: '用 Rust 构建的生产级 Agent 操作系统。按计划 7x24 运行自主 AI Agent，无需用户提示。运行时、安全和渠道基础设施集于一个二进制文件。' },
        { q: '什么是 Hands？', a: '独立的自主能力单元。每个 Hand 配有专属模型、工具和工作流。15 个内置：Clip（视频）、Lead（获客）、Collector（情报）、Predictor（预测）、Researcher、Trader 等。' },
        { q: '支持哪些 LLM 提供商？', a: '50 个提供商：Anthropic、OpenAI、Gemini、Groq、DeepSeek、Mistral、Together、Ollama、vLLM 等，共 200+ 模型。每个 Hand 可配置不同提供商。' },
        { q: '支持哪些渠道？', a: '44 个渠道适配器：Telegram、Discord、Slack、WhatsApp、Signal、Matrix、Teams、Google Chat、飞书、钉钉、Mastodon、Bluesky、LinkedIn、Reddit、IRC 等。' },
        { q: '可以用于生产环境了吗？', a: '2100+ 测试，零 Clippy 警告。16 层安全包括 WASM 沙箱、Merkle 审计链、SSRF 防护。v1.0 之前建议锁定版本。' },
      ],
    },
    community: {
      label: '开源',
      title: '加入社区',
      desc: 'LibreFang 开放构建。贡献代码、报告问题或参与讨论。',
      items: [
        { label: '贡献代码', desc: '提交 PR、修 Bug、完善文档' },
        { label: '报告问题', desc: '发现 Bug？提一个 Issue' },
        { label: '参与讨论', desc: '提问、分享想法' },
        { label: 'Discord', desc: '加入 Discord 服务器' },
      ],
      open: '前往',
    },
    meta: {
      title: 'LibreFang - Agent 操作系统',
      description: 'LibreFang 是用 Rust 构建的生产级 Agent 操作系统。180ms 冷启动，40MB 内存，16 层安全，44 个渠道适配器。7x24 运行自主 AI Agent。',
    },
    workflows: {
      label: '工作流',
      title: '替代整条工作流',
      desc: 'LibreFang 不只是辅助——它会接管。这些是你原本需要雇人来做的工作。',
      items: [
        { title: '内容管道', desc: 'Clip + Twitter 协同：监控趋势视频、剪辑短片、添加字幕、发布社交媒体——全在你离线时完成。' },
        { title: '销售获客', desc: 'Lead 每晚运行：发现潜客、按 ICP 评分、去重、导出干净 CSV。' },
        { title: '竞争情报', desc: 'Collector 监控竞品网站、价格、招聘和新闻，一有变化立即提醒。' },
        { title: '多 Agent 编排', desc: '用工作流链接 Hands：Researcher → Predictor → Clip → 广播到 44 个渠道。' },
        { title: '迁移', desc: '一条命令：librefang migrate --from openclaw。Agent、记忆、技能自动转移。' },
        { title: '生产级安全', desc: 'WASM 沙箱、Merkle 审计链、SSRF 防护、提示注入扫描、GCRA 限流——16 层。' },
      ],
    },
    docs: {
      label: '文档',
      title: '文档',
      desc: 'LibreFang 完整指南',
      categories: [
        { title: '概览', desc: '简介、快速开始、架构' },
        { title: '自动化', desc: '定时任务、Webhooks、集成' },
        { title: '基础设施', desc: '部署、监控、扩容' },
      ],
      viewAll: '查看全部文档',
    },
    githubStats: {
      label: '社区',
      title: '加入社区',
      desc: '帮助我们构建自主 AI 的未来',
      stars: '星标', forks: '分支', issues: '问题', prs: 'PR',
      downloads: '下载量', docsVisits: '文档访问', lastUpdate: '最后更新',
      starHistory: 'Star 趋势', starUs: '给我们星标', discuss: '讨论',
    },
    contributing: {
      label: '贡献',
      title: '如何贡献',
      desc: 'LibreFang 是开源项目，欢迎各种形式的贡献。',
      steps: [
        { title: 'Fork & Clone', desc: 'Fork 仓库并克隆到本地，开始开发。' },
        { title: '选择 Issue', desc: '浏览标记为 "good first issue" 或 "help wanted" 的开放 Issue。' },
        { title: '提交 PR', desc: '完成修改、编写测试，提交 Pull Request 等待审核。' },
      ],
      cta: '阅读贡献指南',
    },
    evolution: {
      label: '技能自我进化',
      title: '会自己学习的 Agent',
      desc: '复杂任务完成后，后台 LLM 会评估本次做法是否值得保存；沉淀下来的技能直接热加载进运行时——无需重启。',
      tagline: '自主沉淀 · 版本留痕 · 安全扫描',
      toolsHeading: '进化工具',
      howItWorks: [
        { title: '自动识别', desc: '一次任务里 5 次以上工具调用会触发后台评审。' },
        { title: '热加载', desc: '新增与更新的技能立即可用，不需要重启守护进程。' },
        { title: '安全扫描', desc: '每次变更都经提示注入检测，命中威胁自动回滚。' },
        { title: '版本历史', desc: '每项技能最多保留 10 个版本，含时间戳、变更说明和回滚快照。' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: '把一次成功的方法沉淀为新的 prompt-only 技能。' },
        { name: 'skill_evolve_update', desc: '整体重写一项技能的提示词内容。' },
        { name: 'skill_evolve_patch', desc: '5 级模糊匹配的精准查找-替换。' },
        { name: 'skill_evolve_rollback', desc: '一键回滚到上一版本。' },
        { name: 'skill_evolve_write_file', desc: '新增 references / templates / scripts / assets 附属文件。' },
        { name: 'skill_evolve_delete', desc: '删除本地创建的技能。' },
      ],
      cta: '查看技能进化文档',
    },
    registry: {
      label: '注册表',
      total: '个条目',
      matching: '个匹配',
      all: '全部',
      searchPlaceholder: '按名称、ID 或标签搜索...',
      loading: '正在加载注册表…',
      errorTitle: '加载注册表失败',
      errorDesc: 'GitHub 限流或代理暂不可用，稍等几秒重试即可。',
      emptyTitle: '此分类暂无内容',
      emptyDesc: '注册表里的这个分类还没有内容，欢迎贡献。',
      contribute: '到 GitHub 提交贡献',
      noMatches: '没有匹配结果：',
      backHome: '首页',
      sourceHint: '数据通过 Cloudflare Worker 从 librefang-registry 仓库代理获取。',
      readDocs: '查看文档',
      manifest: '清单',
      copy: '复制',
      manifestErrorTitle: '无法加载清单',
      allIn: '所有{category}',
      useIt: '如何使用',
      configOnly: '{category}通过 ~/.librefang/config.toml 配置，不走 CLI 安装命令。复制下面的清单，粘贴到配置文件中对应段落即可。',
      relatedIn: '更多{category}',
      retry: '重试',
      openInDashboard: '或在本地仪表盘中安装',
      lastUpdated: '更新于',
      copyLink: '复制此段链接',
      trending: '热门',
      sort: { label: '排序', popular: '热门优先', nameAsc: '名称 A–Z', nameDesc: '名称 Z–A', trending: '点击量' },
      onThisPage: '本页导航',
      previous: '上一个',
      next: '下一个',
      prevNext: '同分类上一个 / 下一个',
      readme: '说明文档',
      viewHistory: '历史',
      templateDiff: '模板差异',
      categories: {
        skills:   { title: '技能', desc: '可插拔的工具包 —— Python、WASM、Node 或 prompt-only 技能，扩展 Agent 的能力边界。' },
        mcp:      { title: 'MCP 服务器', desc: 'Model Context Protocol 服务器，把外部工具与数据直接挂接到任何 Agent。' },
        plugins:  { title: '插件', desc: '运行时扩展 —— 为守护进程添加自定义命令、通道或行为。' },
        hands:    { title: '能力单元', desc: '自主能力单元。每个 Hand 自带模型、工具与工作流 —— 直接启用，不必自己拼装。' },
        agents:   { title: 'Agent 模板', desc: '预置 Agent 模板。模型、系统提示词、能力与调度都写在同一份清单里。' },
        providers:{ title: '模型供应商', desc: 'LLM 供应商适配器：Anthropic、OpenAI、Gemini、Groq、本地模型，以及另外 40 多家。' },
        workflows:{ title: '工作流', desc: '以 TOML 描述的多步 Agent 编排：串联 Agent、条件分支、状态持久化。' },
        channels: { title: '通道', desc: '消息平台适配器：Telegram、Slack、Discord、WhatsApp、LINE 等 44 个。' },
        integrations: { title: '集成', desc: '把 Agent 暴露给主流 SaaS 工具与协议的一方集成。' },
      },
    },
    search: {
      title: '搜索注册表',
      placeholder: '搜索技能、能力单元、Agent、供应商…',
      close: '关闭',
      noResults: '没有匹配 "{query}" 的结果',
      hint: '输入以搜索所有注册表条目。',
      kbd: '↑↓ 导航 · ↵ 打开 · esc 关闭',
      open: '搜索',
    },
    browse: {
      title: '浏览注册表',
      desc: '9 个分类一览 —— 点击任一进入完整清单，按热度排序。',
    },
    notFound: {
      title: '页面未找到',
      desc: '我们没有找到你要找的内容。',
      home: '返回首页',
    },
    pwa: {
      title: '安装 LibreFang',
      desc: '把网站添加到主屏幕 / 程序坞。',
      install: '安装',
      dismiss: '关闭',
    },
    footer: { docs: '文档', license: '许可证', privacy: '隐私', changelog: '更新日志' },
  },

  'zh-TW': {
    nav: { architecture: '架構', hands: '能力單元', performance: '效能', install: '安裝', downloads: '下載', docs: '文件', features: '市場', evolution: '技能自我進化', workflows: '工作流', registry: '註冊表', learnMore: '功能' },
    hero: {
      badge: '開源',
      title1: 'Agent',
      title2: '作業系統',
      typing: [
        '7x24 運行自主 Agent',
        '取代整條工作流',
        '部署到任意硬體',
        '16 層安全防護',
      ],
      desc: 'LibreFang 是面向自主 AI Agent 的生產級執行環境。單一二進位檔案，{handsCount} 個內建能力單元，{channelsCount} 個頻道適配器。Rust 打造，為不能停機的負載而生。',
      getStarted: '開始使用',
      viewGithub: '查看 GitHub',
    },
    stats: { coldStart: '冷啟動', memory: '記憶體', security: '安全層', channels: '頻道', hands: '能力單元', providers: '模型供應商' },
    architecture: {
      label: '系統設計',
      title: '五層架構',
      desc: '從硬體到使用者頻道，每一層都隔離、可測試、可替換。',
      layers: [
        { label: '頻道層', desc: '44 個頻道適配器：Telegram、Slack、Discord、飛書、釘釘、WhatsApp...' },
        { label: '能力層', desc: '15 個自主能力單元，各配專屬模型和工具' },
        { label: '核心層', desc: 'Agent 生命週期、工作流編排、預算控制、排程' },
        { label: '執行環境層', desc: 'Tokio 非同步、WASM 沙箱、Merkle 稽核鏈、SSRF 防護' },
        { label: '硬體層', desc: '單一二進位：筆電、VPS、樹莓派、裸機、雲端' },
      ],
      kernelDescs: ['建立、啟動、暫停、恢復、停止、銷毀', '9 個內建模板，DAG 編排', '單 Agent 限額、全域上限、告警', '基於 Cron 的觸發器、定時任務、事件鉤子', '短期、長期、情景、語意', 'Python 和 Prompt 技能，熱重載', 'Model Context Protocol，Agent 間通訊', 'Open Fang Protocol，網狀網路'],
      runtimeDescs: ['多執行緒非同步執行環境', '不可信程式碼隔離執行', '雜湊鏈完整性驗證', '阻止內部網路存取', '敏感資料流分析', '每 Agent/頻道令牌桶', '多層輸入清洗', '基於角色的存取控制 + 稽核日誌'],
      hardwareDescs: ['32MB，零依賴，複製即執行', 'x86_64 和 ARM64 原生建置', 'ARM64 — Pi 4 上 64MB 記憶體可執行', 'Termux 環境，ARM64 原生', '任意 $5/月 VPS，Docker 可選', '直接部署，無需編排器', '原生桌面應用，系統匣'],
    },
    hands: {
      label: '能力單元',
      title: '15 個內建 Hand',
      desc: '每個 Hand 自帶模型、工具和工作流。啟用即用，無需組裝。',
      items: [
        { name: 'Clip', desc: 'YouTube 影片自動轉直式短影音，AI 字幕，自動發佈到 Telegram。' },
        { name: 'Lead', desc: '每日自主發現潛在客戶，ICP 評分、去重、CSV 匯出。' },
        { name: 'Collector', desc: 'OSINT 級情報監控，變化偵測。' },
        { name: 'Predictor', desc: '校準機率預測引擎，預判市場和業務走向。' },
        { name: 'Researcher', desc: '深度自主研究，來源可信度評分。' },
        { name: 'Trader', desc: '自主市場情報與交易引擎——多訊號分析、對抗推理、風險管理。' },
      ],
      more: '+ 另有 9 個 Hand：Twitter、Browser、Analytics、DevOps、Creator、LinkedIn、Reddit、Strategist、API Tester',
    },
    performance: {
      label: '基準測試',
      title: '生而不同',
      desc: 'Rust 而非 TypeScript。生產級，非原型。',
      metric: '指標',
      others: '其他框架',
      rows: [
        { metric: '冷啟動', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: '閒置記憶體', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: '二進位大小', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: '安全層', others: '2 ~ 3', librefang: '16' },
        { metric: '頻道適配器', others: '8 ~ 15', librefang: '44' },
        { metric: '內建 Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: '開始使用',
      title: '一條指令',
      desc: '單一二進位檔案，無需 Docker，60 秒啟動自主 Agent。',
      terminal: '終端',
      comment: '# Agent 已在自主運行',
      requires: '系統需求',
      includes: '包含內容',
      reqItems: ['Linux / macOS / Windows', '最低 64MB 記憶體', 'x86_64 或 ARM64', 'LLM API 金鑰'],
      incItems: ['{handsCount} 個內建 Hands', '{channelsCount} 個頻道適配器', '{providersCount} 個 LLM 供應商', '桌面應用 (Tauri 2.0)'],
    },
    faq: {
      label: '常見問題',
      title: '常見問題',
      items: [
        { q: '什麼是 LibreFang？', a: '用 Rust 打造的生產級 Agent 作業系統。按排程 7x24 運行自主 AI Agent，無需使用者提示。執行環境、安全和頻道基礎設施集於一個二進位檔案。' },
        { q: '什麼是 Hands？', a: '獨立的自主能力單元。每個 Hand 配有專屬模型、工具和工作流。15 個內建：Clip（影片）、Lead（獲客）、Collector（情報）、Predictor（預測）、Researcher、Trader 等。' },
        { q: '支援哪些 LLM 供應商？', a: '50 個供應商：Anthropic、OpenAI、Gemini、Groq、DeepSeek、Mistral、Together、Ollama、vLLM 等，共 200+ 模型。每個 Hand 可設定不同供應商。' },
        { q: '支援哪些頻道？', a: '44 個頻道適配器：Telegram、Discord、Slack、WhatsApp、Signal、Matrix、Teams、Google Chat、飛書、釘釘、Mastodon、Bluesky、LinkedIn、Reddit、IRC 等。' },
        { q: '可以用於生產環境了嗎？', a: '2100+ 測試，零 Clippy 警告。16 層安全包括 WASM 沙箱、Merkle 稽核鏈、SSRF 防護。v1.0 之前建議鎖定版本。' },
      ],
    },
    community: {
      label: '開源',
      title: '加入社群',
      desc: 'LibreFang 開放打造。貢獻程式碼、回報問題或參與討論。',
      items: [
        { label: '貢獻程式碼', desc: '提交 PR、修 Bug、完善文件' },
        { label: '回報問題', desc: '發現 Bug？提一個 Issue' },
        { label: '參與討論', desc: '提問、分享想法' },
        { label: 'Discord', desc: '加入 Discord 伺服器' },
      ],
      open: '前往',
    },
    meta: {
      title: 'LibreFang - Agent 作業系統',
      description: 'LibreFang 是用 Rust 打造的生產級 Agent 作業系統。180ms 冷啟動，40MB 記憶體，16 層安全，44 個頻道適配器。7x24 運行自主 AI Agent。',
    },
    workflows: {
      label: '工作流',
      title: '取代整條工作流',
      desc: 'LibreFang 不只是輔助——它會接管。這些是你原本需要雇人來做的工作。',
      items: [
        { title: '內容管道', desc: 'Clip + Twitter 協同：監控趨勢影片、剪輯短片、添加字幕、發佈社群媒體——全在你離線時完成。' },
        { title: '銷售獲客', desc: 'Lead 每晚運行：發現潛客、按 ICP 評分、去重、匯出乾淨 CSV。' },
        { title: '競爭情報', desc: 'Collector 監控競品網站、價格、招聘和新聞，一有變化立即提醒。' },
        { title: '多 Agent 編排', desc: '用工作流鏈結 Hands：Researcher → Predictor → Clip → 廣播到 44 個頻道。' },
        { title: '遷移', desc: '一條指令：librefang migrate --from openclaw。Agent、記憶、技能自動轉移。' },
        { title: '生產級安全', desc: 'WASM 沙箱、Merkle 稽核鏈、SSRF 防護、提示注入掃描、GCRA 限流——16 層。' },
      ],
    },
    docs: {
      label: '文件',
      title: '文件',
      desc: 'LibreFang 完整指南',
      categories: [
        { title: '概覽', desc: '簡介、快速開始、架構' },
        { title: '自動化', desc: '排程任務、Webhooks、整合' },
        { title: '基礎設施', desc: '部署、監控、擴容' },
      ],
      viewAll: '查看全部文件',
    },
    githubStats: {
      label: '社群',
      title: '加入社群',
      desc: '幫助我們打造自主 AI 的未來',
      stars: '星標', forks: '分支', issues: '問題', prs: 'PR',
      downloads: '下載量', docsVisits: '文件瀏覽', lastUpdate: '最後更新',
      starHistory: 'Star 趨勢', starUs: '給我們星標', discuss: '討論',
    },
    contributing: {
      label: '貢獻',
      title: '如何貢獻',
      desc: 'LibreFang 是開源專案，歡迎各種形式的貢獻。',
      steps: [
        { title: 'Fork & Clone', desc: 'Fork 儲存庫並複製到本機，開始開發。' },
        { title: '選擇 Issue', desc: '瀏覽標記為 "good first issue" 或 "help wanted" 的開放 Issue。' },
        { title: '提交 PR', desc: '完成修改、撰寫測試，提交 Pull Request 等待審核。' },
      ],
      cta: '閱讀貢獻指南',
    },
    evolution: {
      label: '技能自我進化',
      title: '會自己學習的 Agent',
      desc: '複雜任務結束後，背景 LLM 會評估本次做法是否值得保存；沉澱下來的技能直接熱載入執行階段——不需重啟。',
      tagline: '自主沉澱 · 版本留痕 · 安全掃描',
      toolsHeading: '進化工具',
      howItWorks: [
        { title: '自動識別', desc: '單次任務 5 次以上工具呼叫會觸發背景評審。' },
        { title: '熱載入', desc: '新增與更新的技能立即可用，不需重啟守護程序。' },
        { title: '安全掃描', desc: '每次變更都經提示注入偵測，命中威脅自動回滾。' },
        { title: '版本歷史', desc: '每項技能最多保留 10 個版本，含時間戳、變更說明與回滾快照。' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: '把一次成功的方法沉澱為新的 prompt-only 技能。' },
        { name: 'skill_evolve_update', desc: '整體重寫一項技能的提示詞內容。' },
        { name: 'skill_evolve_patch', desc: '5 級模糊匹配的精準查找-取代。' },
        { name: 'skill_evolve_rollback', desc: '一鍵回滾到上一版本。' },
        { name: 'skill_evolve_write_file', desc: '新增 references / templates / scripts / assets 附屬檔案。' },
        { name: 'skill_evolve_delete', desc: '刪除本機建立的技能。' },
      ],
      cta: '查看技能進化文件',
    },
    registry: {
      label: '註冊表',
      total: '個項目',
      matching: '個匹配',
      all: '全部',
      searchPlaceholder: '依名稱、ID 或標籤搜尋...',
      loading: '正在載入註冊表…',
      errorTitle: '載入註冊表失敗',
      errorDesc: 'GitHub 限流或代理暫時無法使用，稍後重試即可。',
      emptyTitle: '此分類尚無內容',
      emptyDesc: '此分類目前為空，歡迎貢獻。',
      contribute: '到 GitHub 貢獻',
      noMatches: '沒有匹配結果：',
      backHome: '首頁',
      sourceHint: '資料透過 Cloudflare Worker 從 librefang-registry 儲存庫代理取得。',
      readDocs: '閱讀文件',
      manifest: '清單',
      copy: '複製',
      manifestErrorTitle: '無法載入清單',
      allIn: '所有{category}',
      useIt: '如何使用',
      configOnly: '{category}透過 ~/.librefang/config.toml 設定，沒有 CLI 安裝指令。請複製下方清單，貼入設定檔中對應段落。',
      relatedIn: '更多{category}',
      retry: '重試',
      openInDashboard: '或在本地儀表盤中安裝',
      lastUpdated: '更新於',
      copyLink: '複製此段連結',
      trending: '熱門',
      sort: { label: '排序', popular: '熱門優先', nameAsc: '名稱 A–Z', nameDesc: '名稱 Z–A', trending: '點擊量' },
      onThisPage: '本頁導覽',
      previous: '上一個',
      next: '下一個',
      prevNext: '同分類上一個 / 下一個',
      readme: '說明文件',
      viewHistory: '歷史',
      templateDiff: '範本差異',
      categories: {
        skills:   { title: '技能', desc: '可插拔的工具組 —— Python、WASM、Node 或 prompt-only 技能，擴展 Agent 的能力邊界。' },
        mcp:      { title: 'MCP 伺服器', desc: 'Model Context Protocol 伺服器，把外部工具與資料直接接入任何 Agent。' },
        plugins:  { title: '外掛', desc: '執行階段擴充 —— 為守護程序新增自訂命令、通道或行為。' },
        hands:    { title: '能力單元', desc: '自主能力單元。每個 Hand 自帶模型、工具與工作流 —— 直接啟用，不需自己組裝。' },
        agents:   { title: 'Agent 模板', desc: '預置 Agent 模板。模型、系統提示詞、能力與排程都寫在同一份清單裡。' },
        providers:{ title: '模型供應商', desc: 'LLM 供應商介面卡：Anthropic、OpenAI、Gemini、Groq、本地模型，以及另外 40 多家。' },
        workflows:{ title: '工作流', desc: '以 TOML 描述的多步 Agent 編排：串聯 Agent、條件分支、狀態持久化。' },
        channels: { title: '頻道', desc: '訊息平台介面卡：Telegram、Slack、Discord、WhatsApp、LINE 等 44 個。' },
        integrations: { title: '整合', desc: '把 Agent 暴露給主流 SaaS 工具與協定的一方整合。' },
      },
    },
    search: {
      title: '搜尋註冊表',
      placeholder: '搜尋技能、能力單元、Agent、供應商…',
      close: '關閉',
      noResults: '沒有符合 "{query}" 的結果',
      hint: '輸入以搜尋所有註冊表條目。',
      kbd: '↑↓ 瀏覽 · ↵ 開啟 · esc 關閉',
      open: '搜尋',
    },
    browse: {
      title: '瀏覽註冊表',
      desc: '9 個分類一覽 —— 點擊任一進入完整清單，按熱度排序。',
    },
    notFound: {
      title: '頁面未找到',
      desc: '我們沒有找到你要找的內容。',
      home: '返回首頁',
    },
    pwa: {
      title: '安裝 LibreFang',
      desc: '把網站加入主畫面 / 程式塢。',
      install: '安裝',
      dismiss: '關閉',
    },
    footer: { docs: '文件', license: '授權', privacy: '隱私', changelog: '更新日誌' },
  },

  ja: {
    nav: { architecture: 'アーキテクチャ', hands: 'Hands', performance: 'パフォーマンス', install: 'インストール', downloads: 'ダウンロード', docs: 'ドキュメント', features: 'マーケットプレイス', evolution: 'スキル自己進化', workflows: 'ワークフロー', registry: 'レジストリ', learnMore: '機能' },
    hero: {
      badge: 'オープンソース',
      title1: 'Agent',
      title2: 'オペレーティングシステム',
      typing: [
        '自律エージェントを24時間365日稼働',
        'ワークフロー全体を置き換え',
        'あらゆるハードウェアにデプロイ',
        '16層のセキュリティで保護',
      ],
      desc: 'LibreFang は自律型 AI エージェントのための本番グレードランタイムです。シングルバイナリ、{handsCount} の内蔵ケイパビリティユニット、{channelsCount} チャネルアダプタ。ダウンタイムが許されないワークロードのために Rust で構築。',
      getStarted: '始める',
      viewGithub: 'GitHub で見る',
    },
    stats: { coldStart: 'コールドスタート', memory: 'メモリ', security: 'セキュリティ層', channels: 'チャネル', hands: 'Hands', providers: 'プロバイダ' },
    architecture: {
      label: 'システム設計',
      title: '5層アーキテクチャ',
      desc: 'ハードウェアからユーザー向けチャネルまで。各層は分離・テスト・交換可能。',
      layers: [
        { label: 'チャネル層', desc: '44チャネルアダプタ：Telegram、Slack、Discord、Feishu、DingTalk、WhatsApp...' },
        { label: 'Hands層', desc: '15の自律ケイパビリティユニット、専用モデルとツール付き' },
        { label: 'カーネル層', desc: 'エージェントライフサイクル、ワークフロー編成、予算管理、スケジューリング' },
        { label: 'ランタイム層', desc: 'Tokio非同期、WASMサンドボックス、Merkle監査チェーン、SSRF保護' },
        { label: 'ハードウェア層', desc: 'シングルバイナリ：ノートPC、VPS、Raspberry Pi、ベアメタル、クラウド' },
      ],
      kernelDescs: ['作成、開始、一時停止、再開、停止、破棄', '9つの組み込みテンプレート、DAGオーケストレーション', 'エージェント別支出制限、グローバル上限、アラート', 'Cronベースのトリガー、定期タスク、イベントフック', '短期、長期、エピソード、セマンティック', 'Python & Promptスキル、ホットリロード', 'Model Context Protocol、エージェント間通信', 'Open Fang Protocol、メッシュネットワーク'],
      runtimeDescs: ['マルチスレッド非同期ランタイム', '信頼できないコードの隔離実行', 'ハッシュチェーン整合性検証', '内部ネットワークアクセスのブロック', 'シークレットのデータフロー分析', 'エージェント/チャネル毎のトークンバケット', '多層入力サニタイズ', 'ロールベースアクセス制御 + 監査ログ'],
      hardwareDescs: ['32MB、依存関係ゼロ、コピーして実行', 'x86_64 & ARM64ネイティブビルド', 'ARM64 — Pi 4で64MB RAMで動作', 'Termux環境、ARM64ネイティブ', '$5/月のVPS、Docker任意', '直接デプロイ、オーケストレーター不要', 'ネイティブデスクトップアプリ、システムトレイ'],
    },
    hands: {
      label: 'ケイパビリティユニット',
      title: '15の内蔵Hand',
      desc: '各Handは専用のモデル、ツール、ワークフローを搭載。有効化するだけ、組み立て不要。',
      items: [
        { name: 'Clip', desc: 'YouTube動画を縦型ショートに自動変換、AIキャプション付き。Telegramに自動公開。' },
        { name: 'Lead', desc: '毎日の見込み客発見、ICPスコアリング、重複排除、CSV出力。' },
        { name: 'Collector', desc: 'OSINTレベルのインテリジェンス監視、変更検出。' },
        { name: 'Predictor', desc: '校正済み確率予測エンジン、市場と成果を予測。' },
        { name: 'Researcher', desc: 'ソース信頼性スコアリング付きの深層リサーチ。' },
        { name: 'Trader', desc: '自律型マーケットインテリジェンス＆トレーディングエンジン——マルチシグナル分析、敵対的推論、リスク管理。' },
      ],
      more: '+ 他9つのHand：Twitter、Browser、Analytics、DevOps、Creator、LinkedIn、Reddit、Strategist、API Tester',
    },
    performance: {
      label: 'ベンチマーク',
      title: '根本から違う',
      desc: 'TypeScriptではなくRust。プロトタイプではなくプロダクション。',
      metric: '指標',
      others: '他のフレームワーク',
      rows: [
        { metric: 'コールドスタート', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: 'アイドルメモリ', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: 'バイナリサイズ', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: 'セキュリティ層', others: '2 ~ 3', librefang: '16' },
        { metric: 'チャネルアダプタ', others: '8 ~ 15', librefang: '44' },
        { metric: '内蔵Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: '始める',
      title: 'コマンド1つ',
      desc: 'シングルバイナリ。Docker不要。60秒で自律エージェント。',
      terminal: 'ターミナル',
      comment: '# エージェントが自律稼働中',
      requires: '要件',
      includes: '含まれるもの',
      reqItems: ['Linux / macOS / Windows', '最低64MB RAM', 'x86_64 または ARM64', 'LLM API キー'],
      incItems: ['{handsCount}の内蔵Hands', '{channelsCount}チャネルアダプタ', '{providersCount}のLLMプロバイダ', 'デスクトップアプリ (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: 'よくある質問',
      items: [
        { q: 'LibreFangとは？', a: 'Rustで構築された本番グレードのAgent OS。スケジュールに従い24/7自律AIエージェントを実行。ユーザープロンプト不要。ランタイム、セキュリティ、チャネルインフラをシングルバイナリで。' },
        { q: 'Handsとは？', a: '自己完結型の自律ケイパビリティユニット。各Handは専用モデル、ツール、ワークフローを持つ。15内蔵：Clip（動画）、Lead（見込み客）、Collector（OSINT）、Predictor（予測）、Researcher、Trader等。' },
        { q: '対応LLMプロバイダは？', a: '50プロバイダ：Anthropic、OpenAI、Gemini、Groq、DeepSeek、Mistral、Together、Ollama、vLLM等。計200+モデル。各Handで異なるプロバイダを設定可能。' },
        { q: '対応チャネルは？', a: '44チャネルアダプタ：Telegram、Discord、Slack、WhatsApp、Signal、Matrix、Teams、Google Chat、Feishu、DingTalk、Mastodon、Bluesky、LinkedIn、Reddit、IRC等。' },
        { q: '本番利用可能？', a: '2100+テスト、Clippy警告ゼロ。WASMサンドボックス、Merkle監査チェーン、SSRF保護含む16セキュリティ層。v1.0までバージョン固定推奨。' },
      ],
    },
    community: {
      label: 'オープンソース',
      title: 'コミュニティに参加',
      desc: 'LibreFangはオープンに開発。コード貢献、バグ報告、ディスカッションに参加。',
      items: [
        { label: 'コード貢献', desc: 'PR提出、バグ修正、ドキュメント改善' },
        { label: 'バグ報告', desc: 'バグ発見？Issueを開く' },
        { label: 'ディスカッション', desc: '質問やアイデアを共有' },
        { label: 'Discord', desc: 'Discordサーバーに参加' },
      ],
      open: '開く',
    },
    meta: {
      title: 'LibreFang - Agent オペレーティングシステム',
      description: 'LibreFang は Rust で構築された本番グレードの Agent オペレーティングシステムです。180ms コールドスタート、40MB メモリ、16 セキュリティ層、44 チャネルアダプタ。自律 AI エージェントを 24 時間 365 日稼働。',
    },
    workflows: {
      label: 'ワークフロー',
      title: 'ワークフロー全体を置き換え',
      desc: 'LibreFang は単なるアシストではなく、業務を引き継ぎます。これらは本来人を雇って行う作業です。',
      items: [
        { title: 'コンテンツパイプライン', desc: 'Clip + Twitter：トレンド動画を監視、ショート動画をカット、字幕を追加、SNSに公開——すべてオフライン中に完了。' },
        { title: '営業プロスペクティング', desc: 'Lead が毎晩実行：見込み客を発見、ICP適合度でスコアリング、重複排除、クリーンなCSVをエクスポート。' },
        { title: '競合インテリジェンス', desc: 'Collector が競合サイト、価格、求人、ニュースを監視。変化があった瞬間にアラート。' },
        { title: 'マルチエージェントオーケストレーション', desc: 'ワークフローでHandsを連鎖：Researcher → Predictor → Clip → 44チャネルにブロードキャスト。' },
        { title: 'マイグレーション', desc: 'コマンド1つ：librefang migrate --from openclaw。エージェント、メモリ、スキルが自動転送。' },
        { title: '本番セキュリティ', desc: 'WASMサンドボックス、Merkle監査チェーン、SSRF保護、プロンプトインジェクションスキャン、GCRAレート制限——16層。' },
      ],
    },
    docs: {
      label: 'ドキュメント',
      title: 'ドキュメント',
      desc: 'LibreFang の包括的なガイド',
      categories: [
        { title: '概要', desc: '紹介、クイックスタート、アーキテクチャ' },
        { title: '自動化', desc: 'Cronタスク、Webhooks、インテグレーション' },
        { title: 'インフラストラクチャ', desc: 'デプロイ、モニタリング、スケーリング' },
      ],
      viewAll: '全ドキュメントを見る',
    },
    githubStats: {
      label: 'コミュニティ',
      title: 'コミュニティに参加',
      desc: '自律AIエージェントの未来を一緒に築きましょう',
      stars: 'スター', forks: 'フォーク', issues: 'Issue', prs: 'PR',
      downloads: 'ダウンロード', docsVisits: 'ドキュメント閲覧', lastUpdate: '最終更新',
      starHistory: 'スター推移', starUs: 'スターする', discuss: 'ディスカッション',
    },
    contributing: {
      label: 'コントリビュート',
      title: '貢献する方法',
      desc: 'LibreFang はオープンソースであり、あらゆる形の貢献を歓迎します。',
      steps: [
        { title: 'Fork & Clone', desc: 'リポジトリをフォークしてローカルにクローンし、開発を始めましょう。' },
        { title: 'Issueを選ぶ', desc: '"good first issue" や "help wanted" ラベルの付いたオープンIssueを探しましょう。' },
        { title: 'PRを提出', desc: '変更を行い、テストを書き、Pull Requestを提出してレビューを依頼しましょう。' },
      ],
      cta: 'コントリビュートガイドを読む',
    },
    evolution: {
      label: 'スキル自己進化',
      title: '自ら学習する Agent',
      desc: '複雑なタスクの後、バックグラウンドで LLM がその手法を保存する価値があるか評価します。蓄積されたスキルはランタイムに即時ホットロード—再起動不要です。',
      tagline: '自律蓄積 · バージョン管理 · セキュリティスキャン',
      toolsHeading: '進化ツール',
      howItWorks: [
        { title: '自動検出', desc: '5回以上のツール呼び出しで背景評価がトリガーされます。' },
        { title: 'ホットロード', desc: '新規・更新スキルは即座に利用可能。デーモン再起動は不要。' },
        { title: 'セキュリティスキャン', desc: 'すべての変更がプロンプトインジェクション検査を通過し、脅威検出時は自動ロールバック。' },
        { title: 'バージョン履歴', desc: 'スキルごとに最大10バージョン。タイムスタンプ、変更ログ、ロールバックスナップショット付き。' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: '成功した手法を新しい prompt-only スキルとして保存。' },
        { name: 'skill_evolve_update', desc: 'スキルのプロンプトコンテキストを完全に書き換え。' },
        { name: 'skill_evolve_patch', desc: '5段階ファジーマッチングによる精密な検索置換。' },
        { name: 'skill_evolve_rollback', desc: '前バージョンへ即座にロールバック。' },
        { name: 'skill_evolve_write_file', desc: '補助ファイル（references / templates / scripts / assets）を追加。' },
        { name: 'skill_evolve_delete', desc: 'ローカルで作成したスキルを削除。' },
      ],
      cta: 'スキル進化ドキュメントを読む',
    },
    registry: {
      label: 'レジストリ',
      total: '件',
      matching: '件一致',
      all: 'すべて',
      searchPlaceholder: '名前、ID、タグで検索...',
      loading: 'レジストリを読み込み中…',
      errorTitle: 'レジストリを読み込めません',
      errorDesc: 'GitHub のレート制限、またはプロキシが停止中です。数秒後に再試行してください。',
      emptyTitle: 'まだ何もありません',
      emptyDesc: 'レジストリのこのセクションはまだ空です。コントリビュート歓迎。',
      contribute: 'GitHub で貢献',
      noMatches: 'マッチなし:',
      backHome: 'ホーム',
      sourceHint: 'データは Cloudflare Worker を経由して librefang-registry リポジトリから取得しています。',
      readDocs: 'ドキュメントを読む',
      manifest: 'マニフェスト',
      copy: 'コピー',
      manifestErrorTitle: 'マニフェストを読み込めません',
      allIn: 'すべての{category}',
      useIt: '使い方',
      configOnly: '{category}は CLI の install コマンドではなく、~/.librefang/config.toml で設定します。下のマニフェストをコピーして設定ファイルの該当セクションに貼り付けてください。',
      relatedIn: '他の{category}',
      retry: '再試行',
      openInDashboard: 'またはローカルダッシュボードからインストール',
      lastUpdated: '更新',
      copyLink: 'このセクションのリンクをコピー',
      trending: '人気',
      sort: { label: '並び替え', popular: '人気順', nameAsc: '名前 A–Z', nameDesc: '名前 Z–A', trending: 'クリック数' },
      onThisPage: 'このページ',
      previous: '前へ',
      next: '次へ',
      prevNext: 'カテゴリ内の前後',
      readme: 'README',
      viewHistory: '履歴',
      templateDiff: 'テンプレート差分',
      categories: {
        skills:   { title: 'スキル', desc: 'プラグ可能なツールバンドル —— Python、WASM、Node、または prompt-only スキルで Agent の能力を拡張。' },
        mcp:      { title: 'MCP サーバー', desc: 'Model Context Protocol サーバーで、外部ツールとデータソースを任意の Agent に直接接続。' },
        plugins:  { title: 'プラグイン', desc: 'LibreFang デーモンにカスタムコマンド・チャネル・挙動を追加するランタイム拡張。' },
        hands:    { title: 'Hands', desc: '自律的な能力ユニット。各 Hand はモデル、ツール、ワークフローを同梱 —— 組み立てずに有効化。' },
        agents:   { title: 'Agent', desc: 'プリビルトの Agent テンプレート。モデル・システムプロンプト・権限・スケジュールを1つのマニフェストに。' },
        providers:{ title: 'プロバイダー', desc: 'LLM プロバイダーアダプター：Anthropic、OpenAI、Gemini、Groq、ローカル、その他 40+。' },
        workflows:{ title: 'ワークフロー', desc: 'TOML で書かれた多段 Agent オーケストレーション。Agent の連結、条件分岐、状態の永続化。' },
        channels: { title: 'チャネル', desc: 'メッセージングアダプター：Telegram、Slack、Discord、WhatsApp、LINE など 44 プラットフォーム。' },
        integrations: { title: '統合', desc: 'Agent を主要な SaaS ツールとプロトコルに公開するファーストパーティ統合。' },
      },
    },
    search: {
      title: 'レジストリを検索',
      placeholder: 'スキル、Hands、Agent、プロバイダーを検索…',
      close: '閉じる',
      noResults: '"{query}" に一致する結果がありません',
      hint: '入力してレジストリ全体を検索します。',
      kbd: '↑↓ 移動 · ↵ 開く · esc 閉じる',
      open: '検索',
    },
    browse: {
      title: 'レジストリを探す',
      desc: '9 カテゴリを一望、選択して人気順の完全リストへ。',
    },
    notFound: {
      title: 'ページが見つかりません',
      desc: 'お探しのページは存在しません。',
      home: 'ホームに戻る',
    },
    pwa: {
      title: 'LibreFang をインストール',
      desc: 'ホーム画面 / Dock に追加。',
      install: 'インストール',
      dismiss: '閉じる',
    },
    footer: { docs: 'ドキュメント', license: 'ライセンス', privacy: 'プライバシー', changelog: '変更履歴' },
  },

  ko: {
    nav: { architecture: '아키텍처', hands: 'Hands', performance: '성능', install: '설치', downloads: '다운로드', docs: '문서', features: '마켓플레이스', evolution: '스킬 자가 진화', workflows: '워크플로', registry: '레지스트리', learnMore: '기능' },
    hero: {
      badge: '오픈소스',
      title1: 'Agent',
      title2: '운영 체제',
      typing: [
        '자율 에이전트를 24/7 가동',
        '전체 워크플로우를 대체',
        '모든 하드웨어에 배포',
        '16개 보안 레이어로 보호',
      ],
      desc: 'LibreFang은 자율 AI 에이전트를 위한 프로덕션급 런타임입니다. 단일 바이너리, {handsCount}개 내장 기능 유닛, {channelsCount}개 채널 어댑터. 다운타임이 허용되지 않는 워크로드를 위해 Rust로 구축.',
      getStarted: '시작하기',
      viewGithub: 'GitHub 보기',
    },
    stats: { coldStart: '콜드 스타트', memory: '메모리', security: '보안 레이어', channels: '채널', hands: 'Hands', providers: '제공자' },
    architecture: {
      label: '시스템 설계',
      title: '5계층 아키텍처',
      desc: '하드웨어에서 사용자 채널까지. 각 계층은 격리, 테스트, 교체 가능.',
      layers: [
        { label: '채널 계층', desc: '44개 채널 어댑터: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
        { label: 'Hands 계층', desc: '전용 모델과 도구를 갖춘 15개 자율 기능 유닛' },
        { label: '커널 계층', desc: '에이전트 라이프사이클, 워크플로우 오케스트레이션, 예산 제어, 스케줄링' },
        { label: '런타임 계층', desc: 'Tokio 비동기, WASM 샌드박스, Merkle 감사 체인, SSRF 보호' },
        { label: '하드웨어 계층', desc: '단일 바이너리: 노트북, VPS, Raspberry Pi, 베어메탈, 클라우드' },
      ],
      kernelDescs: ['생성, 시작, 일시정지, 재개, 중지, 삭제', '9개 내장 템플릿, DAG 오케스트레이션', '에이전트별 지출 한도, 전역 상한, 알림', 'Cron 기반 트리거, 주기 작업, 이벤트 훅', '단기, 장기, 에피소드, 시맨틱', 'Python & Prompt 스킬, 핫 리로드', 'Model Context Protocol, 에이전트 간 통신', 'Open Fang Protocol, 메시 네트워킹'],
      runtimeDescs: ['멀티스레드 비동기 런타임', '신뢰할 수 없는 코드 격리 실행', '해시 체인 무결성 검증', '내부 네트워크 접근 차단', '시크릿 데이터 흐름 분석', '에이전트/채널별 토큰 버킷', '다중 레이어 입력 새니타이징', '역할 기반 접근 제어 + 감사 로그'],
      hardwareDescs: ['32MB, 제로 의존성, 복사 후 실행', 'x86_64 및 ARM64 네이티브 빌드', 'ARM64 — Pi 4에서 64MB RAM으로 실행', 'Termux 환경, ARM64 네이티브', '$5/월 VPS, Docker 선택', '직접 배포, 오케스트레이터 불필요', '네이티브 데스크톱 앱, 시스템 트레이'],
    },
    hands: {
      label: '기능 유닛',
      title: '15개 내장 Hand',
      desc: '각 Hand는 전용 모델, 도구, 워크플로우를 탑재. 활성화만 하면 됩니다.',
      items: [
        { name: 'Clip', desc: 'YouTube 동영상을 세로형 쇼츠로 자동 변환, AI 자막. Telegram 자동 게시.' },
        { name: 'Lead', desc: '매일 잠재 고객 발견, ICP 점수, 중복 제거, CSV 내보내기.' },
        { name: 'Collector', desc: 'OSINT급 인텔리전스 모니터링, 변경 감지.' },
        { name: 'Predictor', desc: '보정된 확률적 예측 엔진, 시장과 결과 예측.' },
        { name: 'Researcher', desc: '출처 신뢰도 점수가 포함된 심층 리서치.' },
        { name: 'Trader', desc: '자율 시장 인텔리전스 및 거래 엔진 — 다중 신호 분석, 적대적 추론, 리스크 관리.' },
      ],
      more: '+ 9개 추가 Hand: Twitter, Browser, Analytics, DevOps, Creator, LinkedIn, Reddit, Strategist, API Tester',
    },
    performance: {
      label: '벤치마크',
      title: '근본부터 다르다',
      desc: 'TypeScript가 아닌 Rust. 프로토타입이 아닌 프로덕션.',
      metric: '지표',
      others: '다른 프레임워크',
      rows: [
        { metric: '콜드 스타트', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: '유휴 메모리', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: '바이너리 크기', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: '보안 레이어', others: '2 ~ 3', librefang: '16' },
        { metric: '채널 어댑터', others: '8 ~ 15', librefang: '44' },
        { metric: '내장 Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: '시작하기',
      title: '명령어 하나',
      desc: '단일 바이너리. Docker 불필요. 60초면 자율 에이전트 가동.',
      terminal: '터미널',
      comment: '# 에이전트가 자율적으로 실행 중',
      requires: '요구 사항',
      includes: '포함 내용',
      reqItems: ['Linux / macOS / Windows', '최소 64MB RAM', 'x86_64 또는 ARM64', 'LLM API 키'],
      incItems: ['{handsCount}개 내장 Hands', '{channelsCount}개 채널 어댑터', '{providersCount}개 LLM 제공자', '데스크톱 앱 (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: '자주 묻는 질문',
      items: [
        { q: 'LibreFang이란?', a: 'Rust로 구축된 프로덕션급 Agent OS. 스케줄에 따라 24/7 자율 AI 에이전트를 실행. 사용자 프롬프트 불필요. 런타임, 보안, 채널 인프라를 단일 바이너리로.' },
        { q: 'Hands란?', a: '자체 완결형 자율 기능 유닛. 각 Hand는 전용 모델, 도구, 워크플로우를 보유. 15개 내장: Clip(동영상), Lead(영업), Collector(OSINT), Predictor(예측), Researcher, Trader 등.' },
        { q: '지원 LLM 제공자는?', a: '50개 제공자: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM 등. 총 200+ 모델. 각 Hand마다 다른 제공자 설정 가능.' },
        { q: '지원 채널은?', a: '44개 채널 어댑터: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC 등.' },
        { q: '프로덕션 사용 가능?', a: '2100+ 테스트, Clippy 경고 제로. WASM 샌드박스, Merkle 감사 체인, SSRF 보호 포함 16개 보안 레이어. v1.0까지 버전 고정 권장.' },
      ],
    },
    community: {
      label: '오픈소스',
      title: '커뮤니티 참여',
      desc: 'LibreFang은 오픈으로 개발됩니다. 코드 기여, 버그 보고, 토론 참여.',
      items: [
        { label: '코드 기여', desc: 'PR 제출, 버그 수정, 문서 개선' },
        { label: '버그 보고', desc: '버그 발견? Issue 열기' },
        { label: '토론', desc: '질문과 아이디어 공유' },
        { label: 'Discord', desc: 'Discord 서버 참여' },
      ],
      open: '열기',
    },
    meta: {
      title: 'LibreFang - Agent 운영 체제',
      description: 'LibreFang은 Rust로 구축된 프로덕션급 Agent 운영 체제입니다. 180ms 콜드 스타트, 40MB 메모리, 16개 보안 레이어, 44개 채널 어댑터. 자율 AI 에이전트를 24/7 가동.',
    },
    workflows: {
      label: '워크플로우',
      title: '전체 워크플로우를 대체',
      desc: 'LibreFang은 단순 보조가 아닌 업무를 인수합니다. 원래 사람을 고용해야 할 작업들입니다.',
      items: [
        { title: '콘텐츠 파이프라인', desc: 'Clip + Twitter: 트렌드 동영상 모니터링, 쇼츠 편집, 자막 추가, SNS 게시 -- 오프라인 중에도 자동 완료.' },
        { title: '영업 프로스펙팅', desc: 'Lead가 매일 밤 실행: 잠재 고객 발견, ICP 적합도 점수, 중복 제거, 깨끗한 CSV 내보내기.' },
        { title: '경쟁 인텔리전스', desc: 'Collector가 경쟁사 사이트, 가격, 채용, 뉴스를 감시. 변화가 생기는 순간 알림.' },
        { title: '멀티 에이전트 오케스트레이션', desc: '워크플로우로 Hands를 체인: Researcher → Predictor → Clip → 44개 채널로 브로드캐스트.' },
        { title: '마이그레이션', desc: '명령어 하나: librefang migrate --from openclaw. 에이전트, 메모리, 스킬이 자동 이전.' },
        { title: '프로덕션 보안', desc: 'WASM 샌드박스, Merkle 감사 체인, SSRF 보호, 프롬프트 인젝션 스캔, GCRA 속도 제한 -- 16개 레이어.' },
      ],
    },
    docs: {
      label: '문서',
      title: '문서',
      desc: 'LibreFang 종합 가이드',
      categories: [
        { title: '개요', desc: '소개, 빠른 시작, 아키텍처' },
        { title: '자동화', desc: 'Cron 작업, Webhooks, 통합' },
        { title: '인프라', desc: '배포, 모니터링, 스케일링' },
      ],
      viewAll: '전체 문서 보기',
    },
    githubStats: {
      label: '커뮤니티',
      title: '커뮤니티 참여',
      desc: '자율 AI 에이전트의 미래를 함께 만들어 가세요',
      stars: '스타', forks: '포크', issues: '이슈', prs: 'PR',
      downloads: '다운로드', docsVisits: '문서 방문', lastUpdate: '마지막 업데이트',
      starHistory: '스타 추이', starUs: '스타 하기', discuss: '토론',
    },
    contributing: {
      label: '기여',
      title: '기여하는 방법',
      desc: 'LibreFang은 오픈소스이며 모든 형태의 기여를 환영합니다.',
      steps: [
        { title: 'Fork & Clone', desc: '저장소를 포크하고 로컬에 클론하여 개발을 시작하세요.' },
        { title: 'Issue 선택', desc: '"good first issue" 또는 "help wanted" 라벨이 붙은 오픈 Issue를 찾아보세요.' },
        { title: 'PR 제출', desc: '변경을 완료하고 테스트를 작성한 후 Pull Request를 제출하여 리뷰를 요청하세요.' },
      ],
      cta: '기여 가이드 읽기',
    },
    evolution: {
      label: '스킬 자가 진화',
      title: '스스로 배우는 Agent',
      desc: '복잡한 작업 후 백그라운드 LLM이 해당 접근 방식을 저장할 가치가 있는지 평가합니다. 축적된 스킬은 런타임에 즉시 핫 리로드—재시작 불필요.',
      tagline: '자율 축적 · 버전 추적 · 보안 스캔',
      toolsHeading: '진화 도구',
      howItWorks: [
        { title: '자동 감지', desc: '5회 이상 도구 호출 시 백그라운드 리뷰가 트리거됩니다.' },
        { title: '핫 리로드', desc: '신규 및 업데이트된 스킬이 즉시 사용 가능, 데몬 재시작 불필요.' },
        { title: '보안 스캔', desc: '모든 변경이 프롬프트 인젝션 검사를 거치며, 위협 감지 시 자동 롤백.' },
        { title: '버전 기록', desc: '스킬당 최대 10개 버전, 타임스탬프, 변경 로그, 롤백 스냅샷 포함.' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: '성공한 접근 방식을 새 prompt-only 스킬로 저장.' },
        { name: 'skill_evolve_update', desc: '스킬의 프롬프트 컨텍스트를 전체 재작성.' },
        { name: 'skill_evolve_patch', desc: '5단계 퍼지 매칭 기반 정밀 찾기-바꾸기.' },
        { name: 'skill_evolve_rollback', desc: '이전 버전으로 즉시 롤백.' },
        { name: 'skill_evolve_write_file', desc: '보조 파일(references / templates / scripts / assets) 추가.' },
        { name: 'skill_evolve_delete', desc: '로컬에서 생성한 스킬 삭제.' },
      ],
      cta: '스킬 진화 문서 읽기',
    },
    registry: {
      label: '레지스트리',
      total: '항목',
      matching: '개 일치',
      all: '전체',
      searchPlaceholder: '이름, ID 또는 태그로 검색...',
      loading: '레지스트리 로드 중…',
      errorTitle: '레지스트리를 불러올 수 없습니다',
      errorDesc: 'GitHub 레이트 리밋에 걸렸거나 프록시가 다운되었습니다. 잠시 후 다시 시도하세요.',
      emptyTitle: '아직 비어 있습니다',
      emptyDesc: '레지스트리의 이 섹션은 아직 내용이 없습니다. 기여를 환영합니다.',
      contribute: 'GitHub에서 기여하기',
      noMatches: '검색 결과 없음:',
      backHome: '홈',
      sourceHint: 'Cloudflare Worker를 통해 librefang-registry 저장소에서 데이터를 프록시합니다.',
      readDocs: '문서 읽기',
      manifest: '매니페스트',
      copy: '복사',
      manifestErrorTitle: '매니페스트를 불러올 수 없습니다',
      allIn: '모든 {category}',
      useIt: '사용 방법',
      configOnly: '{category}은(는) CLI 설치 명령이 아니라 ~/.librefang/config.toml에서 설정합니다. 아래 매니페스트를 복사해 설정 파일의 해당 섹션에 붙여넣으세요.',
      relatedIn: '더 많은 {category}',
      retry: '다시 시도',
      openInDashboard: '또는 로컬 대시보드에서 설치',
      lastUpdated: '업데이트',
      copyLink: '이 섹션 링크 복사',
      trending: '인기',
      sort: { label: '정렬', popular: '인기순', nameAsc: '이름 A–Z', nameDesc: '이름 Z–A', trending: '클릭수' },
      onThisPage: '이 페이지',
      previous: '이전',
      next: '다음',
      prevNext: '카테고리 내 이전 / 다음',
      readme: 'README',
      viewHistory: '기록',
      templateDiff: '템플릿 차이',
      categories: {
        skills:   { title: '스킬', desc: '플러그 가능한 도구 번들 —— Python, WASM, Node 또는 prompt-only 스킬로 Agent의 능력 확장.' },
        mcp:      { title: 'MCP 서버', desc: 'Model Context Protocol 서버로 외부 도구와 데이터 소스를 모든 Agent에 직접 연결.' },
        plugins:  { title: '플러그인', desc: 'LibreFang 데몬에 커스텀 명령어, 채널, 동작을 추가하는 런타임 확장.' },
        hands:    { title: 'Hands', desc: '자율 능력 유닛. 각 Hand는 자체 모델, 도구, 워크플로를 포함 —— 조립 없이 활성화.' },
        agents:   { title: 'Agent 템플릿', desc: '사전 구축된 Agent 템플릿. 모델, 시스템 프롬프트, 권한, 스케줄을 하나의 매니페스트에.' },
        providers:{ title: '프로바이더', desc: 'LLM 프로바이더 어댑터: Anthropic, OpenAI, Gemini, Groq, 로컬 등 40개 이상.' },
        workflows:{ title: '워크플로', desc: 'TOML로 작성된 다단계 Agent 오케스트레이션. Agent 연결, 조건 분기, 상태 지속성.' },
        channels: { title: '채널', desc: '메시징 어댑터: Telegram, Slack, Discord, WhatsApp, LINE 등 44개 플랫폼.' },
        integrations: { title: '통합', desc: 'Agent를 주요 SaaS 도구와 프로토콜에 노출하는 퍼스트파티 통합.' },
      },
    },
    search: {
      title: '레지스트리 검색',
      placeholder: '스킬, Hand, Agent, 프로바이더 검색…',
      close: '닫기',
      noResults: '"{query}"와 일치하는 결과 없음',
      hint: '입력하여 모든 레지스트리 항목을 검색합니다.',
      kbd: '↑↓ 이동 · ↵ 열기 · esc 닫기',
      open: '검색',
    },
    browse: {
      title: '레지스트리 탐색',
      desc: '9개 카테고리를 한눈에 — 하나를 골라 인기순 전체 목록으로 이동.',
    },
    notFound: {
      title: '페이지를 찾을 수 없습니다',
      desc: '찾으시는 페이지를 찾을 수 없습니다.',
      home: '홈으로',
    },
    pwa: {
      title: 'LibreFang 설치',
      desc: '홈 화면 / Dock에 추가.',
      install: '설치',
      dismiss: '닫기',
    },
    footer: { docs: '문서', license: '라이선스', privacy: '개인정보', changelog: '변경 이력' },
  },

  de: {
    nav: { architecture: 'Architektur', hands: 'Hands', performance: 'Leistung', install: 'Installation', downloads: 'Downloads', docs: 'Dokumentation', features: 'Marktplatz', evolution: 'Skill-Selbstentwicklung', workflows: 'Workflows', registry: 'Registry', learnMore: 'Funktionen' },
    hero: {
      badge: 'Open Source',
      title1: 'Der Agent',
      title2: 'Betriebssystem',
      typing: [
        'autonome Agenten 24/7 betreiben',
        'ganze Workflows ersetzen',
        'auf jeder Hardware deployen',
        '16 Sicherheitsschichten nutzen',
      ],
      desc: 'LibreFang ist eine produktionsreife Laufzeitumgebung für autonome KI-Agenten. Einzelne Binärdatei, {handsCount} eingebaute Fähigkeitseinheiten, {channelsCount} Kanaladapter. In Rust gebaut für Workloads, die nicht ausfallen dürfen.',
      getStarted: 'Loslegen',
      viewGithub: 'Auf GitHub ansehen',
    },
    stats: { coldStart: 'Kaltstart', memory: 'Speicher', security: 'Sicherheitsschichten', channels: 'Kanäle', hands: 'Hands', providers: 'Anbieter' },
    architecture: {
      label: 'Systemdesign',
      title: 'Fünf-Schichten-Architektur',
      desc: 'Von Hardware bis Benutzerkanäle. Jede Schicht ist isoliert, testbar und austauschbar.',
      layers: [
        { label: 'Kanäle', desc: '44 Kanaladapter: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
        { label: 'Hands', desc: '15 autonome Fähigkeitseinheiten mit dedizierten Modellen und Tools' },
        { label: 'Kernel', desc: 'Agent-Lebenszyklus, Workflow-Orchestrierung, Budgetkontrolle, Planung' },
        { label: 'Laufzeit', desc: 'Tokio-Async, WASM-Sandbox, Merkle-Audit-Kette, SSRF-Schutz' },
        { label: 'Hardware', desc: 'Einzelne Binärdatei: Laptop, VPS, Raspberry Pi, Bare Metal, Cloud' },
      ],
      kernelDescs: ['Erstellen, starten, pausieren, fortsetzen, stoppen, zerstören', '9 eingebaute Vorlagen, DAG-Orchestrierung', 'Ausgabelimits pro Agent, globale Obergrenzen, Warnungen', 'Cron-basierte Trigger, Intervall-Tasks, Event-Hooks', 'Kurzzeit, Langzeit, episodisch, semantisch', 'Python & Prompt Skills, Hot-Reload', 'Model Context Protocol, Agent-to-Agent', 'Open Fang Protocol für Mesh-Netzwerk'],
      runtimeDescs: ['Multithreaded asynchrone Laufzeit', 'Isolierte Ausführung für nicht vertrauenswürdigen Code', 'Hash-Chain-Integritätsprüfung', 'Internes Netzwerk blockieren', 'Datenflussanalyse für Geheimnisse', 'Token-Bucket pro Agent/Kanal', 'Mehrstufige Eingabe-Sanitisierung', 'Rollenbasierte Zugriffskontrolle + Audit-Log'],
      hardwareDescs: ['32MB, keine Abhängigkeiten, einfach kopieren und starten', 'x86_64 und ARM64 native Builds', 'ARM64 — läuft auf Pi 4 mit 64MB RAM', 'Termux-Umgebung, ARM64 nativ', 'Jeder $5/Monat VPS, Docker optional', 'Direktes Deployment, kein Orchestrator nötig', 'Native Desktop-App mit System-Tray'],
    },
    hands: {
      label: 'Fähigkeitseinheiten',
      title: '15 eingebaute Hands',
      desc: 'Jede Hand bringt ihr eigenes Modell, Tools und Workflow mit. Aktivieren, nicht zusammenbauen.',
      items: [
        { name: 'Clip', desc: 'YouTube-Videos automatisch in vertikale Shorts umwandeln, mit KI-Untertiteln. Auto-Veröffentlichung auf Telegram.' },
        { name: 'Lead', desc: 'Tägliche Interessenten-Entdeckung mit ICP-Bewertung, Deduplizierung, CSV-Export.' },
        { name: 'Collector', desc: 'OSINT-Niveau Intelligenz-Überwachung mit Änderungserkennung.' },
        { name: 'Predictor', desc: 'Kalibrierte probabilistische Prognose-Engine für Märkte und Ergebnisse.' },
        { name: 'Researcher', desc: 'Tiefenforschung mit Quellenglaubwürdigkeitsbewertung.' },
        { name: 'Trader', desc: 'Autonome Marktintelligenz und Trading-Engine — Multi-Signal-Analyse, adversariales Reasoning, Risikomanagement.' },
      ],
      more: '+ 9 weitere Hands: Twitter, Browser, Analytics, DevOps, Creator, LinkedIn, Reddit, Strategist, API Tester',
    },
    performance: {
      label: 'Benchmarks',
      title: 'Grundlegend anders',
      desc: 'Rust, nicht TypeScript. Produktion, nicht Prototyp.',
      metric: 'Metrik',
      others: 'Andere',
      rows: [
        { metric: 'Kaltstart', others: '2,5 ~ 4s', librefang: '180ms' },
        { metric: 'Leerlaufspeicher', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: 'Binärgröße', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: 'Sicherheitsschichten', others: '2 ~ 3', librefang: '16' },
        { metric: 'Kanaladapter', others: '8 ~ 15', librefang: '44' },
        { metric: 'Eingebaute Hands', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: 'Loslegen',
      title: 'Ein Befehl',
      desc: 'Einzelne Binärdatei. Kein Docker. 60 Sekunden bis zu autonomen Agenten.',
      terminal: 'Terminal',
      comment: '# Agenten laufen jetzt autonom',
      requires: 'Voraussetzungen',
      includes: 'Enthalten',
      reqItems: ['Linux / macOS / Windows', 'Mindestens 64MB RAM', 'x86_64 oder ARM64', 'LLM API-Schlüssel'],
      incItems: ['{handsCount} eingebaute Hands', '{channelsCount} Kanaladapter', '{providersCount} LLM-Anbieter', 'Desktop-App (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: 'Häufige Fragen',
      items: [
        { q: 'Was ist LibreFang?', a: 'Ein produktionsreifes Agent-Betriebssystem in Rust. Führt autonome KI-Agenten 24/7 nach Zeitplan aus — ohne Benutzer-Prompts. Laufzeit, Sicherheit und Kanal-Infrastruktur in einer Binärdatei.' },
        { q: 'Was sind Hands?', a: 'Eigenständige autonome Fähigkeitseinheiten. Jede Hand hat dediziertes Modell, Tools und Workflow. 15 eingebaut: Clip (Video), Lead (Akquise), Collector (OSINT), Predictor (Prognose), Researcher, Trader und mehr.' },
        { q: 'Welche LLM-Anbieter werden unterstützt?', a: '50 Anbieter: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM und mehr. 200+ Modelle insgesamt. Jede Hand kann einen anderen Anbieter nutzen.' },
        { q: 'Welche Kanäle werden unterstützt?', a: '44 Kanaladapter: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC und mehr.' },
        { q: 'Ist es produktionsreif?', a: '2100+ Tests, null Clippy-Warnungen. 16 Sicherheitsschichten inkl. WASM-Sandbox, Merkle-Audit-Kette, SSRF-Schutz. Version bis v1.0 fixieren empfohlen.' },
      ],
    },
    community: {
      label: 'Open Source',
      title: 'Community beitreten',
      desc: 'LibreFang wird offen entwickelt. Code beitragen, Bugs melden oder an Diskussionen teilnehmen.',
      items: [
        { label: 'Beitragen', desc: 'PRs einreichen, Bugs fixen, Docs verbessern' },
        { label: 'Melden', desc: 'Bug gefunden? Issue öffnen' },
        { label: 'Diskutieren', desc: 'Fragen stellen, Ideen teilen' },
        { label: 'Discord', desc: 'Discord-Server beitreten' },
      ],
      open: 'Öffnen',
    },
    meta: {
      title: 'LibreFang - Das Agent-Betriebssystem',
      description: 'LibreFang ist ein produktionsreifes Agent-Betriebssystem, gebaut in Rust. 180ms Kaltstart, 40MB Speicher, 16 Sicherheitsschichten, 44 Kanaladapter. Autonome KI-Agenten rund um die Uhr betreiben.',
    },
    workflows: {
      label: 'Workflows',
      title: 'Ganze Workflows ersetzen',
      desc: 'LibreFang assistiert nicht nur — es übernimmt. Das sind Aufgaben, für die Sie sonst Mitarbeiter einstellen würden.',
      items: [
        { title: 'Content-Pipeline', desc: 'Clip + Twitter: Trend-Videos überwachen, Shorts schneiden, Untertitel hinzufügen, auf Social Media veröffentlichen — alles während Sie offline sind.' },
        { title: 'Vertriebsakquise', desc: 'Lead läuft jede Nacht: Interessenten entdecken, nach ICP bewerten, Duplikate entfernen, saubere CSV exportieren.' },
        { title: 'Wettbewerbsanalyse', desc: 'Collector überwacht Konkurrenz-Websites, Preise, Stellenbörsen und Nachrichten. Alarm bei jeder Änderung.' },
        { title: 'Multi-Agent-Orchestrierung', desc: 'Hands mit Workflow-Orchestrierung verketten: Researcher → Predictor → Clip → Broadcast an 44 Kanäle.' },
        { title: 'Migration', desc: 'Ein Befehl: librefang migrate --from openclaw. Agenten, Speicher und Skills werden automatisch übertragen.' },
        { title: 'Produktionssicherheit', desc: 'WASM-Sandbox, Merkle-Audit-Kette, SSRF-Schutz, Prompt-Injection-Scanning, GCRA-Ratenlimitierung — 16 Schichten.' },
      ],
    },
    docs: {
      label: 'Dokumentation',
      title: 'Dokumentation',
      desc: 'Umfassende Anleitungen für LibreFang',
      categories: [
        { title: 'Überblick', desc: 'Einführung, Schnellstart, Architektur' },
        { title: 'Automatisierung', desc: 'Cron-Aufgaben, Webhooks, Integrationen' },
        { title: 'Infrastruktur', desc: 'Deployment, Monitoring, Skalierung' },
      ],
      viewAll: 'Alle Docs ansehen',
    },
    githubStats: {
      label: 'Community',
      title: 'Community beitreten',
      desc: 'Helfen Sie uns, die Zukunft autonomer KI-Agenten zu gestalten',
      stars: 'Sterne', forks: 'Forks', issues: 'Issues', prs: 'PRs',
      downloads: 'Downloads', docsVisits: 'Docs-Besuche', lastUpdate: 'Letztes Update',
      starHistory: 'Star-Verlauf', starUs: 'Star geben', discuss: 'Diskutieren',
    },
    contributing: {
      label: 'Mitwirken',
      title: 'Wie Sie beitragen können',
      desc: 'LibreFang ist Open Source und begrüßt Beiträge jeder Art.',
      steps: [
        { title: 'Fork & Clone', desc: 'Forken Sie das Repository und klonen Sie es lokal, um loszulegen.' },
        { title: 'Issue auswählen', desc: 'Durchsuchen Sie offene Issues mit den Labels "good first issue" oder "help wanted".' },
        { title: 'PR einreichen', desc: 'Nehmen Sie Änderungen vor, schreiben Sie Tests und reichen Sie einen Pull Request zur Überprüfung ein.' },
      ],
      cta: 'Beitragsrichtlinien lesen',
    },
    evolution: {
      label: 'Skill-Selbstentwicklung',
      title: 'Agents, die sich selbst beibringen',
      desc: 'Nach einer komplexen Aufgabe bewertet ein Hintergrund-LLM, ob der Ansatz gespeichert werden sollte. Neue Skills werden sofort in die Runtime geladen — ohne Neustart.',
      tagline: 'Autonom · Versioniert · Sicherheitsgeprüft',
      toolsHeading: 'Evolution-Tools',
      howItWorks: [
        { title: 'Automatische Erkennung', desc: '5+ Tool-Aufrufe lösen eine Hintergrundüberprüfung aus.' },
        { title: 'Hot-Reload', desc: 'Neue und aktualisierte Skills sind sofort verfügbar — kein Daemon-Neustart.' },
        { title: 'Sicherheitsscan', desc: 'Jede Änderung durchläuft Prompt-Injection-Erkennung mit Auto-Rollback.' },
        { title: 'Versionsverlauf', desc: 'Bis zu 10 Versionen pro Skill mit Zeitstempel, Changelog und Rollback-Snapshots.' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: 'Einen erfolgreichen Ansatz als neuen Prompt-Only-Skill speichern.' },
        { name: 'skill_evolve_update', desc: 'Den Prompt-Kontext eines Skills vollständig neu schreiben.' },
        { name: 'skill_evolve_patch', desc: 'Gezieltes Suchen und Ersetzen mit 5-stufigem Fuzzy-Matching.' },
        { name: 'skill_evolve_rollback', desc: 'Sofort auf die Vorgängerversion zurücksetzen.' },
        { name: 'skill_evolve_write_file', desc: 'Zusatzdateien hinzufügen: References, Templates, Skripte, Assets.' },
        { name: 'skill_evolve_delete', desc: 'Einen lokal erstellten Skill entfernen.' },
      ],
      cta: 'Skill-Evolution-Dokumentation lesen',
    },
    registry: {
      label: 'Registry',
      total: 'Einträge',
      matching: 'Treffer',
      all: 'Alle',
      searchPlaceholder: 'Nach Name, ID oder Tag suchen...',
      loading: 'Registry wird geladen…',
      errorTitle: 'Registry konnte nicht geladen werden',
      errorDesc: 'GitHub-Rate-Limit erreicht oder Proxy nicht erreichbar. In ein paar Sekunden erneut versuchen.',
      emptyTitle: 'Noch leer',
      emptyDesc: 'Dieser Abschnitt der Registry ist noch nicht gefüllt. Beiträge willkommen.',
      contribute: 'Auf GitHub beitragen',
      noMatches: 'Keine Treffer für',
      backHome: 'Startseite',
      sourceHint: 'Daten werden via Cloudflare Worker aus dem librefang-registry Repository geproxyt.',
      readDocs: 'Docs lesen',
      manifest: 'Manifest',
      copy: 'Kopieren',
      manifestErrorTitle: 'Manifest konnte nicht geladen werden',
      allIn: 'Alle {category}',
      useIt: 'Verwenden',
      configOnly: '{category}-Einträge werden über ~/.librefang/config.toml konfiguriert, nicht per CLI-Install-Befehl. Manifest unten kopieren und in den passenden Abschnitt der Config einfügen.',
      relatedIn: 'Mehr {category}',
      retry: 'Erneut versuchen',
      openInDashboard: 'Oder im lokalen Dashboard installieren',
      lastUpdated: 'Aktualisiert',
      copyLink: 'Link zu diesem Abschnitt kopieren',
      trending: 'Beliebt',
      sort: { label: 'Sortieren', popular: 'Beliebt', nameAsc: 'Name A–Z', nameDesc: 'Name Z–A', trending: 'Meistgeklickt' },
      onThisPage: 'Auf dieser Seite',
      previous: 'Zurück',
      next: 'Weiter',
      prevNext: 'Vorheriges / nächstes in der Kategorie',
      readme: 'README',
      viewHistory: 'Verlauf',
      templateDiff: 'Template-Diff',
      categories: {
        skills:   { title: 'Skills', desc: 'Austauschbare Tool-Bundles — Python-, WASM-, Node- oder Prompt-Only-Skills, die die Fähigkeiten eines Agenten erweitern.' },
        mcp:      { title: 'MCP-Server', desc: 'Model-Context-Protocol-Server, die externe Tools und Datenquellen direkt in jeden Agenten einbinden.' },
        plugins:  { title: 'Plugins', desc: 'Runtime-Erweiterungen, die dem LibreFang-Daemon benutzerdefinierte Befehle, Kanäle oder Verhalten hinzufügen.' },
        hands:    { title: 'Hands', desc: 'Autonome Fähigkeitseinheiten. Jede Hand bringt ihr eigenes Modell, Tools und Workflow mit — aktivieren statt zusammenbauen.' },
        agents:   { title: 'Agenten', desc: 'Vorgefertigte Agent-Vorlagen. Modell, System-Prompt, Berechtigungen und Zeitplan in einem Manifest.' },
        providers:{ title: 'Provider', desc: 'LLM-Provider-Adapter: Anthropic, OpenAI, Gemini, Groq, lokal — und 40+ weitere.' },
        workflows:{ title: 'Workflows', desc: 'Mehrstufige Agent-Orchestrierungen in TOML. Agenten verketten, auf Bedingungen verzweigen, Zustand persistieren.' },
        channels: { title: 'Kanäle', desc: 'Messaging-Adapter: Telegram, Slack, Discord, WhatsApp, LINE und 40+ weitere Plattformen.' },
        integrations: { title: 'Integrationen', desc: 'First-Party-Integrationen, die Agenten gängigen SaaS-Tools und -Protokollen aussetzen.' },
      },
    },
    search: {
      title: 'Registry durchsuchen',
      placeholder: 'Skills, Hands, Agents, Provider suchen…',
      close: 'Schließen',
      noResults: 'Keine Treffer für "{query}"',
      hint: 'Tippen, um alle Registry-Einträge zu durchsuchen.',
      kbd: '↑↓ navigieren · ↵ öffnen · esc schließen',
      open: 'Suche',
    },
    browse: {
      title: 'Registry durchsuchen',
      desc: 'Alle 9 Kategorien auf einen Blick — wähle eine für die vollständige Liste, sortiert nach Beliebtheit.',
    },
    notFound: {
      title: 'Seite nicht gefunden',
      desc: 'Wir konnten die gesuchte Seite nicht finden.',
      home: 'Zurück zur Startseite',
    },
    pwa: {
      title: 'LibreFang installieren',
      desc: 'Auf Startbildschirm / Dock hinzufügen.',
      install: 'Installieren',
      dismiss: 'Schließen',
    },
    footer: { docs: 'Dokumentation', license: 'Lizenz', privacy: 'Datenschutz', changelog: 'Changelog' },
  },

  es: {
    nav: { architecture: 'Arquitectura', hands: 'Hands', performance: 'Rendimiento', install: 'Instalar', downloads: 'Descargas', docs: 'Documentación', features: 'Marketplace', evolution: 'Autoevolución de Skills', workflows: 'Flujos de trabajo', registry: 'Registry', learnMore: 'Funciones' },
    hero: {
      badge: 'Código Abierto',
      title1: 'El Agente',
      title2: 'Sistema Operativo',
      typing: [
        'ejecutar agentes autónomos 24/7',
        'reemplazar flujos de trabajo completos',
        'desplegar en cualquier hardware',
        'monitorizar con 16 capas de seguridad',
      ],
      desc: 'LibreFang es un runtime de grado de producción para agentes de IA autónomos. Un solo binario, {handsCount} unidades de capacidad integradas, {channelsCount} adaptadores de canal. Construido en Rust para cargas de trabajo que no pueden caer.',
      getStarted: 'Comenzar',
      viewGithub: 'Ver en GitHub',
    },
    stats: { coldStart: 'Arranque en Frío', memory: 'Memoria', security: 'Capas de Seguridad', channels: 'Canales', hands: 'Hands', providers: 'Proveedores' },
    architecture: {
      label: 'Diseño del Sistema',
      title: 'Arquitectura de cinco capas',
      desc: 'Desde el hardware hasta los canales de usuario. Cada capa está aislada, es testeable y reemplazable.',
      layers: [
        { label: 'Canales', desc: '44 adaptadores de canal: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
        { label: 'Hands', desc: '15 unidades de capacidad autónomas con modelos y herramientas dedicados' },
        { label: 'Kernel', desc: 'Ciclo de vida de agentes, orquestación de workflows, control de presupuesto, programación' },
        { label: 'Runtime', desc: 'Tokio async, sandbox WASM, cadena de auditoría Merkle, protección SSRF' },
        { label: 'Hardware', desc: 'Un solo binario: portátil, VPS, Raspberry Pi, bare metal, nube' },
      ],
      kernelDescs: ['Crear, iniciar, pausar, reanudar, detener, destruir', '9 plantillas integradas, orquestación DAG', 'Límites de gasto por agente, topes globales, alertas', 'Triggers basados en Cron, tareas de intervalo, hooks de eventos', 'Corto plazo, largo plazo, episódica, semántica', 'Skills Python y Prompt, recarga en caliente', 'Model Context Protocol, Agent-to-Agent', 'Open Fang Protocol para redes mesh'],
      runtimeDescs: ['Runtime asíncrono multihilo', 'Ejecución aislada para código no confiable', 'Verificación de integridad por cadena de hash', 'Bloqueo de acceso a red interna', 'Análisis de flujo de datos para secretos', 'Token bucket por agente/canal', 'Sanitización de entrada multicapa', 'Control de acceso basado en roles + log de auditoría'],
      hardwareDescs: ['32MB, cero dependencias, solo copiar y ejecutar', 'Builds nativos x86_64 y ARM64', 'ARM64 — funciona en Pi 4 con 64MB RAM', 'Entorno Termux, ARM64 nativo', 'Cualquier VPS de $5/mes, Docker opcional', 'Despliegue directo, sin orquestador', 'App de escritorio nativa con bandeja del sistema'],
    },
    hands: {
      label: 'Unidades de Capacidad',
      title: '15 Hands integradas',
      desc: 'Cada Hand incluye su propio modelo, herramientas y workflow. Activar, no ensamblar.',
      items: [
        { name: 'Clip', desc: 'Videos de YouTube a shorts verticales con subtítulos IA. Publicación automática en Telegram.' },
        { name: 'Lead', desc: 'Descubrimiento diario de prospectos con puntuación ICP, dedup y exportación CSV.' },
        { name: 'Collector', desc: 'Monitorización de inteligencia nivel OSINT con detección de cambios.' },
        { name: 'Predictor', desc: 'Motor de pronóstico probabilístico calibrado para mercados y resultados.' },
        { name: 'Researcher', desc: 'Investigación profunda con puntuación de credibilidad de fuentes.' },
        { name: 'Trader', desc: 'Motor autónomo de inteligencia de mercado y trading — análisis multi-señal, razonamiento adversarial, gestión de riesgos.' },
      ],
      more: '+ 9 Hands más: Twitter, Browser, Analytics, DevOps, Creator, LinkedIn, Reddit, Strategist, API Tester',
    },
    performance: {
      label: 'Benchmarks',
      title: 'Fundamentalmente diferente',
      desc: 'Rust, no TypeScript. Producción, no prototipo.',
      metric: 'Métrica',
      others: 'Otros',
      rows: [
        { metric: 'Arranque en Frío', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: 'Memoria en Reposo', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: 'Tamaño del Binario', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: 'Capas de Seguridad', others: '2 ~ 3', librefang: '16' },
        { metric: 'Adaptadores de Canal', others: '8 ~ 15', librefang: '44' },
        { metric: 'Hands Integradas', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: 'Comenzar',
      title: 'Un solo comando',
      desc: 'Un solo binario. Sin Docker. 60 segundos para agentes autónomos.',
      terminal: 'terminal',
      comment: '# los agentes están ejecutándose autónomamente',
      requires: 'Requisitos',
      includes: 'Incluye',
      reqItems: ['Linux / macOS / Windows', 'Mínimo 64MB RAM', 'x86_64 o ARM64', 'Clave API LLM'],
      incItems: ['{handsCount} Hands integradas', '{channelsCount} adaptadores de canal', '{providersCount} proveedores LLM', 'App de escritorio (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: 'Preguntas frecuentes',
      items: [
        { q: '¿Qué es LibreFang?', a: 'Un sistema operativo de agentes de grado de producción construido en Rust. Ejecuta agentes de IA autónomos 24/7 por horario — sin prompts de usuario. Runtime, seguridad e infraestructura de canales en un solo binario.' },
        { q: '¿Qué son los Hands?', a: 'Unidades de capacidad autónomas autocontenidas. Cada Hand tiene modelo, herramientas y workflow dedicados. 15 integradas: Clip (video), Lead (prospección), Collector (OSINT), Predictor (pronóstico), Researcher, Trader y más.' },
        { q: '¿Qué proveedores LLM son compatibles?', a: '50 proveedores: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM y más. 200+ modelos en total. Cada Hand puede usar un proveedor diferente.' },
        { q: '¿Qué canales son compatibles?', a: '44 adaptadores de canal: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC y más.' },
        { q: '¿Está listo para producción?', a: '2100+ tests, cero advertencias Clippy. 16 capas de seguridad incluyendo sandbox WASM, cadena de auditoría Merkle, protección SSRF. Fijar versión hasta v1.0 recomendado.' },
      ],
    },
    community: {
      label: 'Código Abierto',
      title: 'Únete a la comunidad',
      desc: 'LibreFang se desarrolla abiertamente. Contribuye código, reporta bugs o únete a la discusión.',
      items: [
        { label: 'Contribuir', desc: 'Enviar PRs, corregir bugs, mejorar docs' },
        { label: 'Reportar', desc: '¿Encontraste un bug? Abre un issue' },
        { label: 'Discutir', desc: 'Haz preguntas, comparte ideas' },
        { label: 'Discord', desc: 'Únete a nuestro servidor de Discord' },
      ],
      open: 'Abrir',
    },
    meta: {
      title: 'LibreFang - Sistema Operativo para Agentes',
      description: 'LibreFang es un sistema operativo de agentes de grado de producción construido en Rust. 180ms de arranque en frío, 40MB de memoria, 16 capas de seguridad, 44 adaptadores de canal. Ejecuta agentes de IA autónomos 24/7.',
    },
    workflows: {
      label: 'Flujos de Trabajo',
      title: 'Reemplaza flujos de trabajo completos',
      desc: 'LibreFang no solo asiste — toma el control. Estas son las operaciones para las que de otro modo contratarías personas.',
      items: [
        { title: 'Pipeline de Contenido', desc: 'Clip + Twitter: monitoriza videos en tendencia, corta shorts, agrega subtítulos, publica en redes — todo mientras estás desconectado.' },
        { title: 'Prospección de Ventas', desc: 'Lead se ejecuta cada noche: descubre prospectos, puntúa por ajuste ICP, elimina duplicados, exporta CSV limpio.' },
        { title: 'Inteligencia Competitiva', desc: 'Collector vigila sitios de la competencia, precios, bolsas de empleo y noticias. Alerta en el momento que algo cambia.' },
        { title: 'Orquestación Multi-Agente', desc: 'Encadena Hands con orquestación de workflows: Researcher → Predictor → Clip → broadcast a 44 canales.' },
        { title: 'Migración', desc: 'Un solo comando: librefang migrate --from openclaw. Agentes, memoria y habilidades se transfieren automáticamente.' },
        { title: 'Seguridad en Producción', desc: 'Sandbox WASM, cadena de auditoría Merkle, protección SSRF, escaneo de inyección de prompts, limitación de tasa GCRA — 16 capas.' },
      ],
    },
    docs: {
      label: 'Documentación',
      title: 'Documentación',
      desc: 'Guías completas para LibreFang',
      categories: [
        { title: 'Descripción General', desc: 'Introducción, inicio rápido, arquitectura' },
        { title: 'Automatización', desc: 'Tareas cron, webhooks, integraciones' },
        { title: 'Infraestructura', desc: 'Despliegue, monitorización, escalado' },
      ],
      viewAll: 'Ver Toda la Documentación',
    },
    githubStats: {
      label: 'Comunidad',
      title: 'Únete a la comunidad',
      desc: 'Ayúdanos a construir el futuro de los agentes de IA autónomos',
      stars: 'Estrellas', forks: 'Forks', issues: 'Issues', prs: 'PRs',
      downloads: 'Descargas', docsVisits: 'Visitas a Docs', lastUpdate: 'Última Actualización',
      starHistory: 'Historial de Estrellas', starUs: 'Danos una Estrella', discuss: 'Discutir',
    },
    contributing: {
      label: 'Contribuir',
      title: 'Cómo contribuir',
      desc: 'LibreFang es código abierto y da la bienvenida a contribuciones de todo tipo.',
      steps: [
        { title: 'Fork & Clone', desc: 'Haz fork del repositorio y clónalo localmente para empezar.' },
        { title: 'Elige un Issue', desc: 'Explora issues abiertos etiquetados como "good first issue" o "help wanted".' },
        { title: 'Envía un PR', desc: 'Realiza tus cambios, escribe tests y envía un pull request para revisión.' },
      ],
      cta: 'Leer Guía de Contribución',
    },
    evolution: {
      label: 'Autoevolución de Skills',
      title: 'Agentes que se enseñan a sí mismos',
      desc: 'Tras una tarea compleja, una revisión LLM en segundo plano decide si el enfoque merece guardarse. Las nuevas skills se cargan en caliente en el runtime — sin reinicio.',
      tagline: 'Autónomo · Versionado · Escaneado',
      toolsHeading: 'Herramientas de evolución',
      howItWorks: [
        { title: 'Detección automática', desc: '5+ llamadas a herramientas disparan una revisión en segundo plano.' },
        { title: 'Recarga en caliente', desc: 'Las skills nuevas o actualizadas están disponibles al instante — sin reiniciar el daemon.' },
        { title: 'Escaneo de seguridad', desc: 'Toda mutación pasa por la detección de prompt injection con rollback automático.' },
        { title: 'Historial de versiones', desc: 'Hasta 10 versiones por skill con timestamps, changelog y snapshots de rollback.' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: 'Guardar un enfoque exitoso como nueva skill prompt-only.' },
        { name: 'skill_evolve_update', desc: 'Reescribir completamente el contexto del prompt de una skill.' },
        { name: 'skill_evolve_patch', desc: 'Buscar y reemplazar preciso con fuzzy matching de 5 estrategias.' },
        { name: 'skill_evolve_rollback', desc: 'Volver a la versión anterior al instante.' },
        { name: 'skill_evolve_write_file', desc: 'Añadir archivos de soporte: references, templates, scripts, assets.' },
        { name: 'skill_evolve_delete', desc: 'Eliminar una skill creada localmente.' },
      ],
      cta: 'Leer Docs de Evolución de Skills',
    },
    registry: {
      label: 'Registry',
      total: 'elementos',
      matching: 'coincidencias',
      all: 'Todos',
      searchPlaceholder: 'Buscar por nombre, ID o etiqueta...',
      loading: 'Cargando registry…',
      errorTitle: 'No se pudo cargar el registry',
      errorDesc: 'Límite de GitHub alcanzado o proxy inactivo. Reintenta en unos segundos.',
      emptyTitle: 'Todavía vacío',
      emptyDesc: 'Esta sección del registry aún no está poblada. Contribuciones bienvenidas.',
      contribute: 'Contribuir en GitHub',
      noMatches: 'Sin resultados para',
      backHome: 'Inicio',
      sourceHint: 'Datos servidos vía Cloudflare Worker desde el repositorio librefang-registry.',
      readDocs: 'Leer la documentación',
      manifest: 'Manifiesto',
      copy: 'Copiar',
      manifestErrorTitle: 'No se pudo cargar el manifiesto',
      allIn: 'Todos los {category}',
      useIt: 'Cómo usar',
      configOnly: 'Los elementos de {category} se configuran mediante ~/.librefang/config.toml en lugar de un comando CLI. Copia el manifiesto y pégalo en la sección correspondiente de tu config.',
      relatedIn: 'Más {category}',
      retry: 'Reintentar',
      openInDashboard: 'O instala desde el dashboard local',
      lastUpdated: 'Actualizado',
      copyLink: 'Copiar enlace a esta sección',
      trending: 'Tendencia',
      sort: { label: 'Ordenar', popular: 'Populares', nameAsc: 'Nombre A–Z', nameDesc: 'Nombre Z–A', trending: 'Más clics' },
      onThisPage: 'En esta página',
      previous: 'Anterior',
      next: 'Siguiente',
      prevNext: 'Anterior / siguiente en la categoría',
      readme: 'README',
      viewHistory: 'Historial',
      templateDiff: 'Diferencias de plantilla',
      categories: {
        skills:   { title: 'Skills', desc: 'Paquetes de herramientas conectables — skills Python, WASM, Node o prompt-only que amplían las capacidades del agente.' },
        mcp:      { title: 'Servidores MCP', desc: 'Servidores Model Context Protocol que conectan herramientas y fuentes de datos externas directamente a cualquier agente.' },
        plugins:  { title: 'Plugins', desc: 'Extensiones en runtime que añaden comandos, canales o comportamientos al daemon LibreFang.' },
        hands:    { title: 'Hands', desc: 'Unidades autónomas de capacidad. Cada Hand trae su propio modelo, herramientas y workflow — actívalo, no lo montes.' },
        agents:   { title: 'Agentes', desc: 'Plantillas de agente listas. Modelo, prompt del sistema, capacidades y schedule en un solo manifiesto.' },
        providers:{ title: 'Proveedores', desc: 'Adaptadores de proveedores LLM: Anthropic, OpenAI, Gemini, Groq, local — y 40+ más.' },
        workflows:{ title: 'Workflows', desc: 'Orquestaciones multi-paso de agentes en TOML. Encadena agentes, bifurca por condiciones, persiste estado.' },
        channels: { title: 'Canales', desc: 'Adaptadores de mensajería: Telegram, Slack, Discord, WhatsApp, LINE y 40+ plataformas más.' },
        integrations: { title: 'Integraciones', desc: 'Integraciones first-party que exponen agentes a herramientas SaaS y protocolos populares.' },
      },
    },
    search: {
      title: 'Buscar en el registry',
      placeholder: 'Buscar skills, hands, agentes, providers…',
      close: 'Cerrar',
      noResults: 'Sin resultados para "{query}"',
      hint: 'Escribe para buscar en todos los elementos del registry.',
      kbd: '↑↓ navegar · ↵ abrir · esc cerrar',
      open: 'Buscar',
    },
    browse: {
      title: 'Explora el registry',
      desc: 'Las 9 categorías de un vistazo — elige una para la lista completa, ordenada por popularidad.',
    },
    notFound: {
      title: 'Página no encontrada',
      desc: 'No pudimos encontrar lo que buscabas.',
      home: 'Volver al inicio',
    },
    pwa: {
      title: 'Instalar LibreFang',
      desc: 'Añádelo a tu pantalla de inicio o dock.',
      install: 'Instalar',
      dismiss: 'Cerrar',
    },
    footer: { docs: 'Documentación', license: 'Licencia', privacy: 'Privacidad', changelog: 'Cambios' },
  },
}
