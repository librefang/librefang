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

export interface RichTextPart {
  text?: string
  code?: string
  em?: string
}

export interface TroubleshootingItem {
  q: string
  lines: RichTextPart[][]
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
    sort?: { label: string; popular: string; nameAsc: string; nameDesc: string; trending: string; downloads?: string; weekly?: string }
    downloads?: string
    stars?: string
    thisWeek?: string
    onThisPage: string
    previous: string
    next: string
    prevNext: string
    readme: string
    viewHistory?: string
    templateDiff?: string
    // Sub-category chips on the category pages (e.g. "communication",
    // "devtools"). The registry stores them as raw English tokens;
    // look them up here for display.
    subcategories?: Record<string, string>
    categories: {
      skills: { title: string; desc: string }
      mcp: { title: string; desc: string }
      plugins: { title: string; desc: string }
      hands: { title: string; desc: string }
      agents: { title: string; desc: string }
      providers: { title: string; desc: string }
      workflows: { title: string; desc: string }
      channels: { title: string; desc: string }
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
  common?: {
    copy: string
    copied: string
    copyToClipboard: string
    loadingReleases: string
    skipToContent: string
    backToTop: string
    viewFull: string
    contributors: string
    agentOs: string
    breadcrumb: string
  }
  header?: {
    toggleTheme: string
    viewSource: string
    source: string
    switchLanguage: string
    switchLanguageNamed: string
    toggleMenu: string
  }
  errorBoundary?: {
    label: string
    title: string
    reload: string
    home: string
  }
  metrics?: {
    breadcrumb: string
    label: string
    title: string
    desc: string
    descPrefix: string
    endpoint: string
    descSuffix: string
    loadError: string
    totalClicks: string
    uniqueItems: string
    activeCategories: string
    generated: string
    perCategory: string
    topItems: string
    items: string
    clicks: string
  }
  deploy?: {
    title: string
    subtitle: string
    terminalIntro: string
    installLocally: string
    website: string
    copyrightSuffix: string
    backToPlatforms: string
    badges: string[]
    tokenRequired: string
    deployFailed: string
    deploying: string
    deployToFly: string
    deployed: string
    deployedDesc: string
    openDashboard: string
    flyConsole: string
    app: string
    region: string
    model: string
    includedModel: string
    upgradeModel: string
    freeNote: string
    stepOneTitle: string
    stepOnePrefix: string
    signUp: string
    or: string
    logIn: string
    stepOneMiddle: string
    personalAccessTokens: string
    stepOneSuffix: string
    stepTwoTitle: string
    stepTwoDesc: string
    tokenLabel: string
    troubleshooting: string
    platforms: Record<string, { desc: string; badge?: string; demo?: string; warning?: string }>
    localInstalls: Record<string, { name: string; desc: string }>
    steps: Record<string, string>
    troubleshootingItems: Record<string, TroubleshootingItem>
  }
  changelog?: {
    title: string
    desc: string
    backHome: string
    releases: string
    downloads: string
    loading: string
    loadError: string
    unknownError: string
    empty: string
    noFilterMatches: string
    fullNotes: string
    viewOnGitHub: string
    filters: Record<'all' | 'stable' | 'prerelease', string>
    releaseTypes: Record<'stable' | 'rc' | 'beta', string>
    changeTypes: Record<'feature' | 'fix' | 'breaking' | 'performance' | 'other', string>
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
  { code: 'pl', name: 'Polski', url: '/pl' },
  { code: 'uk', name: 'Українська', url: '/uk' },
]

export const rawTranslations: Record<string, Translation> = {

  pl: {
    nav: { architecture: 'Architektura', hands: 'Ręce', performance: 'Wydajność', install: 'Instalacja', downloads: 'Pobieranie', docs: 'Dokumentacja', features: 'Rynek', evolution: 'Samodoskonalenie Umiejętności', workflows: 'Przepływy Pracy', registry: 'Rejestr', learnMore: 'Funkcje' },
    hero: {
      badge: 'Open Source',
      title1: 'Agentowy',
      title2: 'System Operacyjny',
      typing: [
        'uruchamiaj autonomicznych agentów 24/7',
        'zastąp całe przepływy pracy',
        'wdrażaj na dowolnym sprzęcie',
        'monitoruj z 16 warstwami bezpieczeństwa',
      ],
      desc: 'LibreFang to środowisko uruchomieniowe klasy produkcyjnej dla autonomicznych agentów AI. Pojedynczy plik binarny, {handsCount} wbudowanych jednostek możliwości, {channelsCount} adapterów kanałów. Zbudowane w Rust dla obciążeń, które nie mogą pozwolić sobie na przestój.',
      getStarted: 'Rozpocznij',
      viewGithub: 'Zobacz na GitHubie',
    },
    stats: { coldStart: 'Zimny Start', memory: 'Pamięć', security: 'Warstwy Bezpieczeństwa', channels: 'Kanały', hands: 'Ręce', providers: 'Dostawcy' },
    architecture: {
      label: 'Projekt Systemu',
      title: 'Pięciowarstwowa architektura',
      desc: 'Od sprzętu po kanały dla użytkowników. Każda warstwa jest odizolowana, testowalna i wymienna.',
      layers: [
        { label: 'Kanały', desc: '44 adaptery: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
        { label: 'Ręce', desc: '15 autonomicznych jednostek możliwości z dedykowanymi modelami i narzędziami' },
        { label: 'Jądro', desc: 'Cykl życia agenta, orkiestracja przepływów pracy, kontrola budżetu, harmonogramowanie' },
        { label: 'Środowisko Uruchomieniowe', desc: 'Asynchroniczne Tokio, piaskownica WASM, łańcuch audytów Merkle, ochrona SSRF' },
        { label: 'Sprzęt', desc: 'Pojedynczy plik binarny: laptop, VPS, Raspberry Pi, serwery dedykowane, chmura' },
      ],
      kernelDescs: [
        'Twórz, uruchamiaj, wstrzymuj, wznawiaj, zatrzymuj, niszcz',
        '9 wbudowanych szablonów, orkiestracja DAG',
        'Limity wydatków na agenta, globalne limity, alerty',
        'Wyzwalacze oparte na Cronie, zadania interwałowe, haki zdarzeń',
        'Krótkoterminowa, długoterminowa, epizodyczna, semantyczna, auto-konsolidacja pamięci',
        'Umiejętności oparte na Pythonie i promptach, hot-reload',
        'Model Context Protocol, Agent-to-Agent',
        'Open Fang Protocol dla sieci mesh',
      ],
      runtimeDescs: [
        'Wielowątkowe środowisko asynchroniczne',
        'Izolowane wykonywanie niezaufanego kodu',
        'Weryfikacja integralności oparta na łańcuchu hashów',
        'Blokowanie dostępu do sieci wewnętrznej',
        'Analiza przepływu danych pod kątem sekretów',
        'Wiadra tokenów na agenta/kanał',
        'Wielowarstwowa sanityzacja wejścia',
        'Kontrola dostępu oparta na rolach + dziennik audytu',
      ],
      hardwareDescs: [
        '32MB, brak zależności, po prostu skopiuj i uruchom',
        'Natywne kompilacje dla x86_64 i ARM64',
        'ARM64 — działa na Pi 4 z 64MB RAM',
        'Środowisko Termux, natywne dla ARM64',
        'Dowolny VPS za $5/mc, Docker opcjonalnie',
        'Bezpośrednie wdrożenie, nie wymaga orkiestratora',
        'Natywna aplikacja desktopowa z zasobnikiem systemowym',
      ],
    },
    hands: {
      label: 'Jednostki Możliwości',
      title: '15 wbudowanych Rąk',
      desc: 'Każda Ręka jest dostarczana z własnym modelem, narzędziami i przepływem pracy. Aktywuj, nie składaj.',
      items: [
        { name: 'Clip', desc: 'Z YouTube na pionowe shorty z napisami AI. Automatyczna publikacja na Telegramie.' },
        { name: 'Lead', desc: 'Codzienne odkrywanie potencjalnych klientów z oceną ICP, deduplikacją i eksportem CSV.' },
        { name: 'Collector', desc: 'Monitorowanie wywiadu klasy OSINT z wykrywaniem zmian.' },
        { name: 'Predictor', desc: 'Skalibrowane prognozowanie probabilistyczne dla rynków i wyników.' },
        { name: 'Researcher', desc: 'Głębokie badania z oceną wiarygodności źródeł.' },
        { name: 'Trader', desc: 'Autonomiczny silnik analizy rynkowej i handlowej — analiza wielosygnałowa, wnioskowanie kontradyktoryjne, zarządzanie ryzykiem.' },
      ],
      more: '+ 9 więcej: Twitter, Przeglądarka, Analityka, DevOps, Twórca, LinkedIn, Reddit, Strateg, Tester API',
    },
    performance: {
      label: 'Benchmarki',
      title: 'Zbudowany inaczej',
      desc: 'Rust, nie TypeScript. Produkcja, nie prototyp.',
      metric: 'Metryka',
      others: 'Inne',
      rows: [
        { metric: 'Zimny Start', others: '2.5 ~ 4s', librefang: '180ms' },
        { metric: 'Pamięć w Spoczynku', others: '180 ~ 250MB', librefang: '40MB' },
        { metric: 'Rozmiar Pliku Binarnego', others: '100 ~ 200MB', librefang: '32MB' },
        { metric: 'Warstwy Bezpieczeństwa', others: '2 ~ 3', librefang: '16' },
        { metric: 'Adaptery Kanałów', others: '8 ~ 15', librefang: '44' },
        { metric: 'Wbudowane Ręce', others: '0', librefang: '15' },
      ],
    },
    install: {
      label: 'Rozpocznij',
      title: 'Jedno polecenie',
      desc: 'Pojedynczy plik binarny. Bez Dockera. 60 sekund do autonomicznych agentów.',
      terminal: 'terminal',
      comment: '# agenci teraz działają autonomicznie',
      requires: 'Wymaga',
      includes: 'Zawiera',
      reqItems: ['Linux / macOS / Windows', 'Minimum 64MB RAM', 'x86_64 lub ARM64', 'Klucz API LLM'],
      incItems: ['{handsCount} wbudowanych Rąk', '{channelsCount} adapterów kanałów', '{providersCount} dostawców LLM', 'Aplikacja desktopowa (Tauri 2.0)'],
    },
    faq: {
      label: 'FAQ',
      title: 'Częste pytania',
      items: [
        { q: 'Czym jest LibreFang?', a: 'Agentowy System Operacyjny klasy produkcyjnej zbudowany w Rust. Uruchamia autonomicznych agentów AI zgodnie z harmonogramem 24/7 — bez wymogu wprowadzania promptów przez użytkownika. Środowisko uruchomieniowe, bezpieczeństwo i infrastruktura kanałów w jednym pliku binarnym.' },
        { q: 'Czym są Ręce (Hands)?', a: 'Samodzielne, autonomiczne jednostki możliwości. Każda Ręka posiada dedykowany model, narzędzia i przepływ pracy. 15 wbudowanych: Clip (wideo), Lead (poszukiwanie klientów), Collector (OSINT), Predictor (prognozowanie), Researcher, Trader i więcej.' },
        { q: 'Którzy dostawcy LLM są obsługiwani?', a: '50 dostawców: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM i inni. Ponad 200 modeli ogółem. Każda Ręka może używać innego dostawcy.' },
        { q: 'Jakie kanały są obsługiwane?', a: '44 adaptery kanałów: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC i inne.' },
        { q: 'Czy jest gotowy do produkcji?', a: 'Ponad 2100 testów, zero ostrzeżeń Clippy. 16 warstw bezpieczeństwa, w tym piaskownica WASM, łańcuch audytów Merkle, ochrona przed SSRF. Zablokuj wersję aż do v1.0, aby zachować stabilność.' },
      ],
    },
    community: {
      label: 'Open Source',
      title: 'Dołącz do społeczności',
      desc: 'LibreFang jest rozwijany otwarcie. Wnoś kod, zgłaszaj problemy lub dołącz do dyskusji.',
      items: [
        { label: 'Współtwórz', desc: 'Przesyłaj PR, naprawiaj błędy, ulepszaj dokumentację' },
        { label: 'Zgłoś', desc: 'Znalazłeś błąd? Otwórz zgłoszenie' },
        { label: 'Dyskutuj', desc: 'Zadawaj pytania, dziel się pomysłami' },
        { label: 'Discord', desc: 'Dołącz do naszego serwera Discord' },
      ],
      open: 'Otwórz',
    },
    meta: {
      title: 'LibreFang - Agentowy System Operacyjny',
      description: 'LibreFang to Agentowy System Operacyjny klasy produkcyjnej zbudowany w Rust. 180ms zimnego startu, 40MB pamięci, 16 warstw bezpieczeństwa, 44 adapterów kanałów. Uruchamiaj autonomicznych agentów AI 24/7.',
    },
    workflows: {
      label: 'Przepływy Pracy',
      title: 'Zastąp całe przepływy pracy',
      desc: 'LibreFang nie tylko asystuje — przejmuje kontrolę. To są operacje, do których w przeciwnym razie musiałbyś zatrudnić ludzi.',
      items: [
        { title: 'Linia Produkcji Treści', desc: 'Clip + Twitter: monitoruj zyskujące popularność filmy, tnij na shorty, dodawaj napisy, publikuj w mediach społecznościowych — wszystko, gdy jesteś offline.' },
        { title: 'Poszukiwanie Klientów', desc: 'Lead uruchamia się co noc: znajduje potencjalnych klientów, ocenia ich dopasowanie do ICP, usuwa duplikaty, eksportuje czysty plik CSV.' },
        { title: 'Wywiad Konkurencyjny', desc: 'Collector obserwuje strony konkurencji, cenniki, ogłoszenia o pracę i wiadomości. Powiadamia natychmiast, gdy coś się zmieni.' },
        { title: 'Orkiestracja Wieloagentowa', desc: 'Łącz Ręce w przepływy pracy: Researcher → Predictor → Clip → nadawaj na 44 kanałach.' },
        { title: 'Migracja', desc: 'Jedno polecenie: librefang migrate --from openclaw. Agenci, pamięć i umiejętności przenoszą się automatycznie.' },
        { title: 'Bezpieczeństwo Produkcyjne', desc: 'Piaskownica WASM, łańcuch audytów Merkle, ochrona przed SSRF, skanowanie wstrzykiwania promptów, limitowanie zapytań GCRA — 16 warstw.' },
      ],
    },
    docs: {
      label: 'Dokumentacja',
      title: 'Dokumentacja',
      desc: 'Kompleksowe przewodniki po LibreFang',
      categories: [
        { title: 'Przegląd', desc: 'Wprowadzenie, szybki start, architektura' },
        { title: 'Automatyzacja', desc: 'Zadania Cron, webhooki, integracje' },
        { title: 'Infrastruktura', desc: 'Wdrażanie, monitorowanie, skalowanie' },
      ],
      viewAll: 'Wyświetl Całą Dokumentację',
    },
    githubStats: {
      label: 'Społeczność',
      title: 'Dołącz do społeczności',
      desc: 'Pomóż nam budować przyszłość autonomicznych agentów AI',
      stars: 'Gwiazdki', forks: 'Forki', issues: 'Problemy', prs: 'PR-y',
      downloads: 'Pobrania', docsVisits: 'Odwiedziny Dokumentacji', lastUpdate: 'Ostatnia Aktualizacja',
      starHistory: 'Historia Gwiazdek', starUs: 'Daj nam Gwiazdkę', discuss: 'Dyskutuj',
    },
    contributing: {
      label: 'Współpraca',
      title: 'Jak wnieść wkład',
      desc: 'LibreFang to projekt open source i chętnie przyjmuje wszelkie formy wkładu.',
      steps: [
        { title: 'Fork i Clone', desc: 'Rozwidl repozytorium i sklonuj je lokalnie, aby rozpocząć.' },
        { title: 'Wybierz Zgłoszenie', desc: 'Przeglądaj otwarte zgłoszenia oznaczone jako "good first issue" lub "help wanted".' },
        { title: 'Prześlij PR', desc: 'Wprowadź zmiany, napisz testy i prześlij Pull Request (PR) do recenzji.' },
      ],
      cta: 'Przeczytaj Przewodnik Dla Kontrybutorów',
    },
    evolution: {
      label: 'Samodoskonalenie Umiejętności',
      title: 'Agenci, którzy uczą się sami',
      desc: 'Po wykonaniu skomplikowanego zadania, w tle odbywa się analiza przez LLM, która decyduje, czy podejście warto zachować. Nowe umiejętności są ładowane do środowiska uruchomieniowego w locie — bez restartu.',
      tagline: 'Autonomiczne. Z wersjonowaniem. Skanowane pod kątem bezpieczeństwa.',
      toolsHeading: 'Narzędzia ewolucji',
      howItWorks: [
        { title: 'Automatyczne wykrywanie', desc: 'Ponad 5 wywołań narzędzi uruchamia w tle przegląd podejścia.' },
        { title: 'Hot-reload', desc: 'Nowe i zaktualizowane umiejętności są dostępne od razu — bez restartowania demona.' },
        { title: 'Skanowanie bezpieczeństwa', desc: 'Każda modyfikacja przechodzi przez detekcję wstrzykiwania promptów z automatycznym cofaniem zmian.' },
        { title: 'Historia wersji', desc: 'Do 10 wersji na umiejętność ze znacznikami czasu, rejestrem zmian i migawkami przywracania.' },
      ],
      tools: [
        { name: 'skill_evolve_create', desc: 'Zapisz udane podejście jako nową umiejętność opartą wyłącznie na prompcie.' },
        { name: 'skill_evolve_update', desc: 'Całkowicie przepisz kontekst prompta danej umiejętności.' },
        { name: 'skill_evolve_patch', desc: 'Docelowe wyszukiwanie i zamiana z pięcioma strategiami dopasowywania rozmytego.' },
        { name: 'skill_evolve_rollback', desc: 'Błyskawicznie przywróć poprzednią wersję.' },
        { name: 'skill_evolve_write_file', desc: 'Dodaj pliki pomocnicze: referencje, szablony, skrypty, zasoby.' },
        { name: 'skill_evolve_delete', desc: 'Usuń lokalnie utworzoną umiejętność.' },
      ],
      cta: 'Przeczytaj Dokumentację Ewolucji Umiejętności',
    },
    registry: {
      label: 'Rejestr',
      total: 'łącznie',
      matching: 'pasujących',
      all: 'Wszystkie',
      searchPlaceholder: 'Szukaj według nazwy, id lub tagu...',
      loading: 'Ładowanie rejestru…',
      errorTitle: 'Nie udało się załadować rejestru',
      errorDesc: 'Osiągnięto limit zapytań API GitHub lub serwer proxy nie działa. Spróbuj ponownie za kilka sekund.',
      emptyTitle: 'Nic tu jeszcze nie ma',
      emptyDesc: 'Ta sekcja rejestru nie została jeszcze uzupełniona. Zajrzyj tu wkrótce lub stwórz własną zawartość.',
      contribute: 'Współtwórz na GitHubie',
      noMatches: 'Brak wyników dla',
      backHome: 'Strona Główna',
      sourceHint: 'Dane pochodzą z repozytorium librefang-registry na GitHubie.',
      readDocs: 'Przeczytaj dokumentację',
      manifest: 'Manifest',
      copy: 'Kopiuj',
      manifestErrorTitle: 'Nie udało się załadować manifestu',
      allIn: 'Wszystkie {category}',
      useIt: 'Użyj',
      configOnly: 'Wpisy {category} są konfigurowane w pliku ~/.librefang/config.toml, a nie za pomocą polecenia instalacji w CLI. Skopiuj poniższy manifest i wklej go do pasującej sekcji w swojej konfiguracji.',
      relatedIn: 'Więcej {category}',
      retry: 'Ponów',
      openInDashboard: 'Lub zainstaluj przez lokalny panel sterowania',
      lastUpdated: 'Zaktualizowano',
      copyLink: 'Skopiuj link do tej sekcji',
      trending: 'Na czasie',
      downloads: 'pobrań',
      stars: 'gwiazdek',
      thisWeek: 'w tym tygodniu',
      sort: { label: 'Sortuj', popular: 'Popularne', nameAsc: 'Nazwa A–Z', nameDesc: 'Nazwa Z–A', trending: 'Najczęściej klikane', downloads: 'Najczęściej pobierane', weekly: 'Trendy w tym tygodniu' },
      onThisPage: 'Na tej stronie',
      previous: 'Poprzednia',
      next: 'Następna',
      prevNext: 'Poprzednia / następna w kategorii',
      readme: 'README',
      viewHistory: 'Historia',
      templateDiff: 'Różnica szablonów (Diff)',
      subcategories: {
        ai: 'AI', business: 'Biznes', cloud: 'Chmura', communication: 'Komunikacja',
        content: 'Treść', creation: 'Tworzenie', data: 'Dane', developer: 'Programista',
        development: 'Programowanie', devtools: 'Narzędzia Deweloperskie', email: 'E-mail',
        engineering: 'Inżynieria', enterprise: 'Przedsiębiorstwo', iot: 'IoT',
        language: 'Język', messaging: 'Wiadomości', productivity: 'Produktywność',
        research: 'Badania', skills: 'Umiejętności', social: 'Społeczność', thinking: 'Myślenie',
      },
      categories: {
        skills: { title: 'Umiejętności', desc: 'Możliwe do podłączenia pakiety narzędzi — Python, WASM, Node lub tylko bazujące na promptach, które rozszerzają to, co może zrobić agent.' },
        mcp:    { title: 'Serwery MCP', desc: 'Serwery Model Context Protocol, które bezpośrednio łączą zewnętrzne narzędzia i źródła danych z dowolnym agentem.' },
        plugins:{ title: 'Wtyczki', desc: 'Rozszerzenia środowiska uruchomieniowego, które dodają własne polecenia, kanały lub zachowania do demona LibreFang.' },
        hands:  { title: 'Ręce', desc: 'Autonomiczne jednostki możliwości. Każda Ręka ma własny model, narzędzia i przepływ pracy — aktywuj, nie składaj.' },
        agents: { title: 'Agenci', desc: 'Gotowe szablony agentów. Model, prompt systemowy, funkcje i harmonogram — wszystko w jednym manifeście.' },
        providers: { title: 'Dostawcy', desc: 'Adaptery dostawców LLM. Anthropic, OpenAI, Gemini, Groq, lokalne — i ponad 40 innych.' },
        workflows: { title: 'Przepływy Pracy', desc: 'Wielokrokowa orkiestracja agentów wyrażona w TOML. Łącz agentów, twórz warunki rozgałęziające, przechowuj stan.' },
        channels:  { title: 'Kanały', desc: 'Adaptery do przesyłania wiadomości. Telegram, Slack, Discord, WhatsApp, LINE i ponad 40 innych platform.' },
      },
    },
    search: {
      title: 'Przeszukaj rejestr',
      placeholder: 'Szukaj umiejętności, rąk, agentów, dostawców…',
      close: 'Zamknij',
      noResults: 'Brak wyników dla "{query}"',
      hint: 'Zacznij pisać, aby przeszukać wszystkie wpisy w rejestrze.',
      kbd: '↑↓ nawigacja · ↵ otwórz · esc zamknij',
      open: 'Szukaj',
    },
    browse: {
      title: 'Przeglądaj rejestr',
      desc: 'Wszystkie kategorie na pierwszy rzut oka — wybierz jedną, aby zobaczyć wszystkie wpisy posortowane według popularności.',
    },
    notFound: {
      title: 'Nie znaleziono strony',
      desc: "Nie mogliśmy znaleźć tego, czego szukasz.",
      home: 'Wróć do strony głównej',
    },
    pwa: {
      title: 'Zainstaluj LibreFang',
      desc: 'Dodaj stronę do ekranu głównego lub paska zadań.',
      install: 'Zainstaluj',
      dismiss: 'Odrzuć',
    },
    common: {
      copy: 'Kopiuj',
      copied: 'Skopiowano!',
      copyToClipboard: 'Kopiuj do schowka',
      loadingReleases: 'Wczytywanie wydań...',
      skipToContent: 'Przejdź do treści',
      backToTop: 'Powrót do góry',
      viewFull: 'Zobacz całość',
      contributors: 'Współtwórcy',
      agentOs: 'Agent OS',
      breadcrumb: 'Ścieżka nawigacji',
    },
    header: {
      toggleTheme: 'Przełącz motyw',
      viewSource: 'Zobacz kod źródłowy na GitHub',
      source: 'Kod źródłowy',
      switchLanguage: 'Zmień język',
      switchLanguageNamed: 'Zmień język ({name})',
      toggleMenu: 'Przełącz menu',
    },
    errorBoundary: {
      label: 'Błąd wykonania',
      title: 'Coś poszło nie tak.',
      reload: 'Odśwież',
      home: 'Strona główna',
    },
    metrics: {
      breadcrumb: 'Metryki',
      label: 'Metryki rejestru',
      title: 'Telemetria kliknięć',
      desc: 'Zagregowane liczby wyświetleń stron szczegółów w całym rejestrze, rejestrowane przez endpoint workera /api/registry/click.',
      descPrefix: 'Zagregowane liczby wyświetleń stron szczegółów w całym rejestrze, rejestrowane przez endpoint workera ',
      endpoint: '/api/registry/click',
      descSuffix: '.',
      loadError: 'Nie udało się wczytać metryk',
      totalClicks: 'Łączna liczba kliknięć',
      uniqueItems: 'Unikalne pozycje',
      activeCategories: 'Aktywne kategorie',
      generated: 'Wygenerowano',
      perCategory: 'Według kategorii',
      topItems: 'Najpopularniejsze pozycje ogółem',
      items: 'pozycji',
      clicks: 'kliknięć',
    },
    deploy: {
      title: 'Wdróż LibreFang',
      subtitle: 'Wybierz swoją platformę',
      terminalIntro: 'Albo wdróż z terminala:',
      installLocally: 'Zainstaluj lokalnie',
      website: 'Strona internetowa',
      copyrightSuffix: 'Agent Operating System',
      backToPlatforms: 'Powrót do platform',
      badges: ['Darmowy LLM w zestawie', 'Nie wymaga klucza API', '1 GB przestrzeni dyskowej'],
      tokenRequired: 'Podaj swój token API Fly.io.',
      deployFailed: 'Wdrożenie nie powiodło się',
      deploying: 'Wdrażanie...',
      deployToFly: 'Wdróż na Fly.io',
      deployed: 'Wdrożono!',
      deployedDesc: 'Twoja instancja LibreFang uruchamia się (1-2 min). Darmowy LLM (Step 3.5 Flash) jest już skonfigurowany i gotowy do użycia.',
      openDashboard: 'Otwórz panel',
      flyConsole: 'Konsola Fly.io',
      app: 'Aplikacja',
      region: 'Region',
      model: 'Model',
      includedModel: 'Step 3.5 Flash (darmowy)',
      upgradeModel: 'Ulepsz model',
      freeNote: 'Darmowy LLM (Step 3.5 Flash przez OpenRouter) jest już skonfigurowany. Twoja instancja działa od razu — bez konieczności podawania kluczy API.',
      stepOneTitle: 'Uzyskaj token API Fly.io',
      stepOnePrefix: '',
      signUp: 'Zarejestruj się',
      or: 'lub',
      logIn: 'zaloguj się',
      stepOneMiddle: 'na Fly.io, następnie przejdź do',
      personalAccessTokens: 'Personal Access Tokens',
      stepOneSuffix: 'i utwórz nowy token.',
      stepTwoTitle: 'Wklej i wdróż',
      stepTwoDesc: 'Twój token jest wysyłany wyłącznie do API Fly.io i nigdy nie jest przechowywany na naszych serwerach.',
      tokenLabel: 'Token API Fly.io',
      troubleshooting: 'Rozwiązywanie problemów',
      platforms: {
        flyio: {
          desc: 'Zawsze darmowe, trwałe przechowywanie danych',
          badge: 'Polecane',
          demo: 'Demo na żywo',
        },
        render: {
          desc: 'Wdrożenie OAuth jednym kliknięciem',
          badge: 'Najprościej',
          demo: 'Demo na żywo',
          warning: 'Darmowy plan: usypia po 15 min, brak trwałego przechowywania danych',
        },
        railway: {
          desc: 'Proste wdrożenie z darmowym kredytem 5 USD',
        },
        gcp: {
          desc: 'Zawsze darmowe (e2-micro), 30 GB przechowywania danych',
          badge: 'Terraform',
        },
        docker: {
          desc: 'Jedna komenda, działa wszędzie',
        },
      },
      localInstalls: {
        macos: {
          name: 'macOS',
          desc: 'Homebrew albo pobierz plik binarny',
        },
        linux: {
          name: 'Linux',
          desc: 'Skrypt instalacyjny albo pobierz plik binarny',
        },
        windows: {
          name: 'Windows',
          desc: 'Instalator PowerShell albo plik .msi',
        },
      },
      steps: {
        auth: 'Weryfikowanie tokenu...',
        app: 'Tworzenie aplikacji...',
        net: 'Przydzielanie adresów IP...',
        vol: 'Tworzenie trwałego wolumenu...',
        machine: 'Uruchamianie maszyny ze Step 3.5 Flash...',
      },
      troubleshootingItems: {
        sso: {
          q: 'Nie można utworzyć Personal Access Token (błąd SSO)',
          lines: [
            [
              {
                text: 'Jeśli widzisz: ',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: 'Użyj tokenu przypisanego do organizacji. Uruchom w terminalu:',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: 'Następnie wklej wygenerowany token powyżej.',
              },
            ],
          ],
        },
        image: {
          q: 'Wdrożenie nie powiodło się: obraz nie znaleziony',
          lines: [
            [
              {
                text: 'Obraz Docker ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' jest budowany przy każdym wydaniu.',
              },
            ],
            [
              {
                text: 'Jeśli żadne wydanie nie zostało jeszcze opublikowane, użyj poniższego skryptu wdrożenia z terminala — buduje on obraz ze źródeł.',
              },
            ],
          ],
        },
        llm: {
          q: 'Jak dodać lub zmienić dostawcę LLM po wdrożeniu?',
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: 'Następnie edytuj ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' za pomocą ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ', aby zmienić domyślny model.',
              },
            ],
          ],
        },
      },
    },
    changelog: {
      title: 'Dziennik zmian',
      desc: 'Śledź każdą aktualizację LibreFang',
      backHome: 'Powrót do strony głównej',
      releases: 'wydania',
      downloads: 'pobrania',
      loading: 'Wczytywanie wydań...',
      loadError: 'Nie udało się wczytać wydań',
      unknownError: 'Nieznany błąd',
      empty: 'Nie znaleziono żadnych wydań.',
      noFilterMatches: 'Brak wydań spełniających bieżący filtr.',
      fullNotes: 'Zobacz pełne informacje o wydaniu na GitHub.',
      viewOnGitHub: 'Zobacz na GitHub',
      filters: {
        all: 'Wszystkie',
        stable: 'Stabilne',
        prerelease: 'Wersje przedwydaniowe',
      },
      releaseTypes: {
        stable: 'Stabilna',
        rc: 'RC',
        beta: 'Beta',
      },
      changeTypes: {
        feature: 'Dodano',
        fix: 'Naprawiono',
        breaking: 'Zmiany łamiące kompatybilność',
        performance: 'Wydajność',
        other: 'Inne',
      },
    },
    footer: { docs: 'Dokumentacja', license: 'Licencja', privacy: 'Prywatność', changelog: 'Dziennik zmian' },
  },
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
        'Short-term, long-term, episodic, semantic, auto-dream consolidation',
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
      downloads: 'downloads',
      stars: 'stars',
      thisWeek: 'this week',
      sort: { label: 'Sort', popular: 'Popular', nameAsc: 'Name A–Z', nameDesc: 'Name Z–A', trending: 'Most clicked', downloads: 'Most downloaded', weekly: 'Trending this week' },
      onThisPage: 'On this page',
      previous: 'Previous',
      next: 'Next',
      prevNext: 'Previous / next in category',
      readme: 'README',
      viewHistory: 'History',
      templateDiff: 'Template diff',
      subcategories: {
        ai: 'AI', business: 'Business', cloud: 'Cloud', communication: 'Communication',
        content: 'Content', creation: 'Creation', data: 'Data', developer: 'Developer',
        development: 'Development', devtools: 'DevTools', email: 'Email',
        engineering: 'Engineering', enterprise: 'Enterprise', iot: 'IoT',
        language: 'Language', messaging: 'Messaging', productivity: 'Productivity',
        research: 'Research', skills: 'Skills', social: 'Social', thinking: 'Thinking',
      },
      categories: {
        skills: { title: 'Skills', desc: 'Pluggable tool bundles — Python, WASM, Node, or prompt-only skills that extend what an agent can do.' },
        mcp:    { title: 'MCP Servers', desc: 'Model Context Protocol servers that plug external tools and data sources directly into any agent.' },
        plugins:{ title: 'Plugins', desc: 'Runtime extensions that add custom commands, channels, or behaviors to the LibreFang daemon.' },
        hands:  { title: 'Hands', desc: 'Autonomous capability units. Each Hand ships with its own model, tools, and workflow — activate, don\'t assemble.' },
        agents: { title: 'Agents', desc: 'Pre-built agent templates. Model, system prompt, capabilities and scheduling — all in one manifest.' },
        providers: { title: 'Providers', desc: 'LLM provider adapters. Anthropic, OpenAI, Gemini, Groq, local — and the 40+ in between.' },
        workflows: { title: 'Workflows', desc: 'Multi-step agent orchestrations expressed as TOML. Chain agents, branch on conditions, persist state.' },
        channels:  { title: 'Channels', desc: 'Messaging adapters. Telegram, Slack, Discord, WhatsApp, LINE and 40+ other platforms.' },
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
    common: {
      copy: 'Copy',
      copied: 'Copied!',
      copyToClipboard: 'Copy to clipboard',
      loadingReleases: 'Loading releases...',
      skipToContent: 'Skip to content',
      backToTop: 'Back to top',
      viewFull: 'View Full',
      contributors: 'Contributors',
      agentOs: 'Agent OS',
      breadcrumb: 'Breadcrumb',
    },
    header: {
      toggleTheme: 'Toggle theme',
      viewSource: 'View source on GitHub',
      source: 'Source',
      switchLanguage: 'Switch language',
      switchLanguageNamed: 'Switch language ({name})',
      toggleMenu: 'Toggle menu',
    },
    errorBoundary: {
      label: 'Runtime error',
      title: 'Something went wrong.',
      reload: 'Reload',
      home: 'Home',
    },
    metrics: {
      breadcrumb: 'Metrics',
      label: 'Registry Metrics',
      title: 'Click telemetry',
      desc: 'Aggregate counts of detail-page views across the registry, recorded via the /api/registry/click worker endpoint.',
      descPrefix: 'Aggregate counts of detail-page views across the registry, recorded via the ',
      endpoint: '/api/registry/click',
      descSuffix: ' worker endpoint.',
      loadError: 'Could not load metrics',
      totalClicks: 'Total clicks',
      uniqueItems: 'Unique items',
      activeCategories: 'Active categories',
      generated: 'Generated',
      perCategory: 'Per category',
      topItems: 'Top items overall',
      items: 'items',
      clicks: 'clicks',
    },
    deploy: {
      title: 'Deploy LibreFang',
      subtitle: 'Choose your platform',
      terminalIntro: 'Or deploy from your terminal:',
      installLocally: 'Install locally',
      website: 'Website',
      copyrightSuffix: 'Agent Operating System',
      backToPlatforms: 'Back to platforms',
      badges: ['Free LLM included', 'No API key needed', '1 GB storage'],
      tokenRequired: 'Please enter your Fly.io API Token.',
      deployFailed: 'Deployment failed',
      deploying: 'Deploying...',
      deployToFly: 'Deploy to Fly.io',
      deployed: 'Deployed!',
      deployedDesc: 'Your LibreFang instance is starting up (1-2 min). Free LLM (Step 3.5 Flash) is pre-configured and ready to use.',
      openDashboard: 'Open Dashboard',
      flyConsole: 'Fly.io Console',
      app: 'App',
      region: 'Region',
      model: 'Model',
      includedModel: 'Step 3.5 Flash (free)',
      upgradeModel: 'Upgrade model',
      freeNote: 'A free LLM (Step 3.5 Flash via OpenRouter) is pre-configured. Your instance works out of the box — no API keys required.',
      stepOneTitle: 'Get a Fly.io API Token',
      stepOnePrefix: '',
      signUp: 'Sign up',
      or: 'or',
      logIn: 'log in',
      stepOneMiddle: 'to Fly.io, then go to',
      personalAccessTokens: 'Personal Access Tokens',
      stepOneSuffix: 'and create a new token.',
      stepTwoTitle: 'Paste and deploy',
      stepTwoDesc: 'Your token is only sent to the Fly.io API and is never stored on our servers.',
      tokenLabel: 'Fly.io API Token',
      troubleshooting: 'Troubleshooting',
      platforms: {
        flyio: { desc: 'Free forever, persistent storage', badge: 'Recommended', demo: 'Live Demo' },
        render: { desc: 'One-click OAuth deploy', badge: 'Easiest', demo: 'Live Demo', warning: 'Free tier: sleeps after 15 min, no persistent storage' },
        railway: { desc: 'Simple deploy with $5 free credit' },
        gcp: { desc: 'Free forever (e2-micro), 30GB storage', badge: 'Terraform' },
        docker: { desc: 'One command, runs anywhere' },
      },
      localInstalls: {
        macos: { name: 'macOS', desc: 'Homebrew or download binary' },
        linux: { name: 'Linux', desc: 'Install script or download binary' },
        windows: { name: 'Windows', desc: 'PowerShell installer or .msi' },
      },
      steps: {
        auth: 'Verifying token...',
        app: 'Creating app...',
        net: 'Allocating IP addresses...',
        vol: 'Creating persistent volume...',
        machine: 'Launching machine with Step 3.5 Flash...',
      },
      troubleshootingItems: {
        sso: {
          q: 'Cannot create Personal Access Token (SSO error)',
          lines: [
            [
              { text: 'If you see: ' },
              { em: '"Access Tokens cannot be created because an organization requires SSO"' },
            ],
            [{ text: 'Use a per-org token instead. Run in your terminal:' }],
            [{ code: 'flyctl tokens org <your-org-name>' }],
            [{ text: 'Then paste the generated token above.' }],
          ],
        },
        image: {
          q: 'Deploy failed: image not found',
          lines: [
            [
              { text: 'The Docker image ' },
              { code: 'ghcr.io/librefang/librefang:latest' },
              { text: ' is built on each release.' },
            ],
            [{ text: 'If no release has been published yet, use the terminal deploy script below — it builds from source.' }],
          ],
        },
        llm: {
          q: 'How to add or change LLM provider after deploy?',
          lines: [
            [{ code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name' }],
            [
              { text: 'Then edit ' },
              { code: '/data/config.toml' },
              { text: ' via ' },
              { code: 'flyctl ssh console' },
              { text: ' to update the default model.' },
            ],
          ],
        },
      },
    },
    changelog: {
      title: 'Changelog',
      desc: 'Track every update to LibreFang',
      backHome: 'Back to home',
      releases: 'releases',
      downloads: 'downloads',
      loading: 'Loading releases...',
      loadError: 'Failed to load releases',
      unknownError: 'Unknown error',
      empty: 'No releases found.',
      noFilterMatches: 'No releases match the current filter.',
      fullNotes: 'See full release notes on GitHub.',
      viewOnGitHub: 'View on GitHub',
      filters: { all: 'All', stable: 'Stable', prerelease: 'Pre-release' },
      releaseTypes: { stable: 'Stable', rc: 'RC', beta: 'Beta' },
      changeTypes: {
        feature: 'Added',
        fix: 'Fixed',
        breaking: 'Breaking',
        performance: 'Performance',
        other: 'Other',
      },
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
      kernelDescs: ['创建、启动、暂停、恢复、停止、销毁', '9 个内置模板，DAG 编排', '单 Agent 限额、全局上限、告警', '基于 Cron 的触发器、定时任务、事件钩子', '短期、长期、情景、语义，梦境自动整固', 'Python 和 Prompt 技能，热重载', 'Model Context Protocol，Agent 间通信', 'Open Fang Protocol，网状网络'],
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
      sourceHint: '数据代理自 GitHub 上的 librefang-registry 仓库。',
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
      downloads: '次下载',
      stars: '颗星',
      thisWeek: '本周新增',
      sort: { label: '排序', popular: '热门优先', nameAsc: '名称 A–Z', nameDesc: '名称 Z–A', trending: '点击量最多', downloads: '下载量最多', weekly: '本周热门' },
      onThisPage: '本页导航',
      previous: '上一个',
      next: '下一个',
      prevNext: '同分类上一个 / 下一个',
      readme: '说明文档',
      viewHistory: '历史',
      templateDiff: '模板差异',
      subcategories: {
        ai: 'AI', business: '商业', cloud: '云', communication: '通信',
        content: '内容', creation: '创作', data: '数据', developer: '开发者',
        development: '开发', devtools: '开发者工具', email: '邮件',
        engineering: '工程', enterprise: '企业', iot: '物联网',
        language: '语言', messaging: '消息', productivity: '生产力',
        research: '研究', skills: '技能', social: '社交', thinking: '思考',
      },
      categories: {
        skills:   { title: '技能', desc: '可插拔的工具包 —— Python、WASM、Node 或 prompt-only 技能，扩展 Agent 的能力边界。' },
        mcp:      { title: 'MCP 服务器', desc: 'Model Context Protocol 服务器，把外部工具与数据直接挂接到任何 Agent。' },
        plugins:  { title: '插件', desc: '运行时扩展 —— 为守护进程添加自定义命令、渠道或行为。' },
        hands:    { title: '能力单元', desc: '自主能力单元。每个 Hand 自带模型、工具与工作流 —— 直接启用，不必自己拼装。' },
        agents:   { title: 'Agent 模板', desc: '预置 Agent 模板。模型、系统提示词、能力与调度都写在同一份清单里。' },
        providers:{ title: '模型供应商', desc: 'LLM 供应商适配器：Anthropic、OpenAI、Gemini、Groq、本地模型，以及另外 40 多家。' },
        workflows:{ title: '工作流', desc: '以 TOML 描述的多步 Agent 编排：串联 Agent、条件分支、状态持久化。' },
        channels: { title: '渠道', desc: '消息平台适配器：Telegram、Slack、Discord、WhatsApp、LINE 等 44 个。' },
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
      desc: '每个分类一览 —— 点击任一进入完整清单，按热度排序。',
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
    common: {
      copy: '复制',
      copied: '已复制！',
      copyToClipboard: '复制到剪贴板',
      loadingReleases: '正在加载版本...',
      skipToContent: '跳转到主要内容',
      backToTop: '返回顶部',
      viewFull: '查看完整内容',
      contributors: '贡献者',
      agentOs: 'Agent OS',
      breadcrumb: '面包屑导航',
    },
    header: {
      toggleTheme: '切换主题',
      viewSource: '在 GitHub 上查看源码',
      source: '源码',
      switchLanguage: '切换语言',
      switchLanguageNamed: '切换语言（{name}）',
      toggleMenu: '切换菜单',
    },
    errorBoundary: {
      label: '运行时错误',
      title: '出错了。',
      reload: '重新加载',
      home: '首页',
    },
    metrics: {
      breadcrumb: '指标',
      label: '注册表指标',
      title: '点击遥测数据',
      desc: '通过 /api/registry/click 工作端点记录的注册表详情页浏览量汇总统计。',
      descPrefix: '通过 ',
      endpoint: '/api/registry/click',
      descSuffix: ' 工作端点记录的注册表详情页浏览量汇总统计。',
      loadError: '无法加载指标',
      totalClicks: '总点击数',
      uniqueItems: '独立项目数',
      activeCategories: '活跃分类数',
      generated: '生成时间',
      perCategory: '按分类统计',
      topItems: '总体热门项目',
      items: '项',
      clicks: '次点击',
    },
    deploy: {
      title: '部署 LibreFang',
      subtitle: '选择你的平台',
      terminalIntro: '或者从终端部署：',
      installLocally: '本地安装',
      website: '官网',
      copyrightSuffix: 'Agent Operating System',
      backToPlatforms: '返回平台列表',
      badges: ['内置免费 LLM', '无需 API 密钥', '1 GB 存储空间'],
      tokenRequired: '请输入你的 Fly.io API Token。',
      deployFailed: '部署失败',
      deploying: '正在部署...',
      deployToFly: '部署到 Fly.io',
      deployed: '部署成功！',
      deployedDesc: '你的 LibreFang 实例正在启动（1-2 分钟）。免费 LLM（Step 3.5 Flash）已预先配置好，可直接使用。',
      openDashboard: '打开仪表盘',
      flyConsole: 'Fly.io 控制台',
      app: '应用',
      region: '区域',
      model: '模型',
      includedModel: 'Step 3.5 Flash（免费）',
      upgradeModel: '升级模型',
      freeNote: '已预先配置免费 LLM（通过 OpenRouter 提供的 Step 3.5 Flash）。你的实例开箱即用，无需任何 API 密钥。',
      stepOneTitle: '获取 Fly.io API Token',
      stepOnePrefix: '',
      signUp: '注册',
      or: '或',
      logIn: '登录',
      stepOneMiddle: 'Fly.io，然后前往',
      personalAccessTokens: 'Personal Access Tokens',
      stepOneSuffix: '创建一个新的 Token。',
      stepTwoTitle: '粘贴并部署',
      stepTwoDesc: '你的 Token 只会发送给 Fly.io API，绝不会存储在我们的服务器上。',
      tokenLabel: 'Fly.io API Token',
      troubleshooting: '故障排查',
      platforms: {
        flyio: {
          desc: '永久免费，持久化存储',
          badge: '推荐',
          demo: '在线演示',
        },
        render: {
          desc: '一键 OAuth 部署',
          badge: '最简单',
          demo: '在线演示',
          warning: '免费套餐：15 分钟后休眠，无持久化存储',
        },
        railway: {
          desc: '部署简单，附带 5 美元免费额度',
        },
        gcp: {
          desc: '永久免费（e2-micro），30GB 存储空间',
          badge: 'Terraform',
        },
        docker: {
          desc: '一条命令，随处运行',
        },
      },
      localInstalls: {
        macos: {
          name: 'macOS',
          desc: '使用 Homebrew 或下载二进制文件',
        },
        linux: {
          name: 'Linux',
          desc: '使用安装脚本或下载二进制文件',
        },
        windows: {
          name: 'Windows',
          desc: 'PowerShell 安装程序或 .msi',
        },
      },
      steps: {
        auth: '正在验证 Token...',
        app: '正在创建应用...',
        net: '正在分配 IP 地址...',
        vol: '正在创建持久化卷...',
        machine: '正在启动搭载 Step 3.5 Flash 的机器...',
      },
      troubleshootingItems: {
        sso: {
          q: '无法创建 Personal Access Token（SSO 错误）',
          lines: [
            [
              {
                text: '如果你看到：',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: '请改用按组织生成的 Token。在终端中运行：',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: '然后将生成的 Token 粘贴到上方。',
              },
            ],
          ],
        },
        image: {
          q: '部署失败：找不到镜像',
          lines: [
            [
              {
                text: 'Docker 镜像 ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' 会在每次发布时构建。',
              },
            ],
            [
              {
                text: '如果尚未发布任何版本，请使用下方的终端部署脚本——它会从源码构建。',
              },
            ],
          ],
        },
        llm: {
          q: '部署后如何添加或更改 LLM 提供商？',
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: '然后通过 ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ' 编辑 ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' 以更新默认模型。',
              },
            ],
          ],
        },
      },
    },
    changelog: {
      title: '更新日志',
      desc: '跟踪 LibreFang 的每一次更新',
      backHome: '返回首页',
      releases: '个版本',
      downloads: '次下载',
      loading: '正在加载版本...',
      loadError: '加载版本失败',
      unknownError: '未知错误',
      empty: '未找到任何版本。',
      noFilterMatches: '没有符合当前筛选条件的版本。',
      fullNotes: '查看 GitHub 上的完整发布说明。',
      viewOnGitHub: '在 GitHub 上查看',
      filters: {
        all: '全部',
        stable: '稳定版',
        prerelease: '预发布版',
      },
      releaseTypes: {
        stable: '稳定版',
        rc: 'RC',
        beta: 'Beta',
      },
      changeTypes: {
        feature: '新增',
        fix: '修复',
        breaking: '破坏性变更',
        performance: '性能',
        other: '其他',
      },
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
      kernelDescs: ['建立、啟動、暫停、恢復、停止、銷毀', '9 個內建模板，DAG 編排', '單 Agent 限額、全域上限、告警', '基於 Cron 的觸發器、定時任務、事件鉤子', '短期、長期、情景、語意，夢境自動整固', 'Python 和 Prompt 技能，熱重載', 'Model Context Protocol，Agent 間通訊', 'Open Fang Protocol，網狀網路'],
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
      openInDashboard: '或在本地儀表板中安裝',
      lastUpdated: '更新於',
      copyLink: '複製此段連結',
      trending: '熱門',
      downloads: '次下載',
      stars: '顆星',
      thisWeek: '本週',
      sort: { label: '排序', popular: '熱門優先', nameAsc: '名稱 A–Z', nameDesc: '名稱 Z–A', trending: '點擊量最多', downloads: '下載量最多', weekly: '本週熱門' },
      onThisPage: '本頁導覽',
      previous: '上一個',
      next: '下一個',
      prevNext: '同分類上一個 / 下一個',
      readme: '說明文件',
      viewHistory: '歷史',
      templateDiff: '範本差異',
      subcategories: {
        ai: 'AI', business: '商業', cloud: '雲端', communication: '通訊',
        content: '內容', creation: '創作', data: '資料', developer: '開發者',
        development: '開發', devtools: '開發者工具', email: '郵件',
        engineering: '工程', enterprise: '企業', iot: '物聯網',
        language: '語言', messaging: '訊息', productivity: '生產力',
        research: '研究', skills: '技能', social: '社交', thinking: '思考',
      },
      categories: {
        skills:   { title: '技能', desc: '可插拔的工具組 —— Python、WASM、Node 或 prompt-only 技能，擴展 Agent 的能力邊界。' },
        mcp:      { title: 'MCP 伺服器', desc: 'Model Context Protocol 伺服器，把外部工具與資料直接接入任何 Agent。' },
        plugins:  { title: '外掛', desc: '執行階段擴充 —— 為守護程序新增自訂命令、通道或行為。' },
        hands:    { title: '能力單元', desc: '自主能力單元。每個 Hand 自帶模型、工具與工作流 —— 直接啟用，不需自己組裝。' },
        agents:   { title: 'Agent 模板', desc: '預置 Agent 模板。模型、系統提示詞、能力與排程都寫在同一份清單裡。' },
        providers:{ title: '模型供應商', desc: 'LLM 供應商介面卡：Anthropic、OpenAI、Gemini、Groq、本地模型，以及另外 40 多家。' },
        workflows:{ title: '工作流', desc: '以 TOML 描述的多步 Agent 編排：串聯 Agent、條件分支、狀態持久化。' },
        channels: { title: '頻道', desc: '訊息平台介面卡：Telegram、Slack、Discord、WhatsApp、LINE 等 44 個。' },
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
      desc: '各分類一覽 —— 點擊任一進入完整清單，按熱度排序。',
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
    common: {
      agentOs: 'Agent 作業系統',
      backToTop: '回到頂部',
      breadcrumb: '導覽路徑',
      contributors: '貢獻者',
      copied: '已複製！',
      copy: '複製',
      copyToClipboard: '複製到剪貼簿',
      loadingReleases: '正在載入版本…',
      skipToContent: '跳至主要內容',
      viewFull: '查看完整內容',
    },
    header: {
      source: '原始碼',
      switchLanguage: '切換語言',
      switchLanguageNamed: '切換語言（{name}）',
      toggleMenu: '切換選單',
      toggleTheme: '切換主題',
      viewSource: '在 GitHub 上查看原始碼',
    },
    errorBoundary: {
      home: '首頁',
      label: '執行階段錯誤',
      reload: '重新載入',
      title: '發生錯誤。',
    },
    metrics: {
      activeCategories: '活躍分類',
      breadcrumb: '指標',
      clicks: '次點擊',
      desc: '透過 /api/registry/click worker 端點記錄的整個註冊表詳情頁面瀏覽次數統計。',
      descPrefix: '透過 ',
      descSuffix: ' worker 端點記錄的整個註冊表詳情頁面瀏覽次數統計。',
      endpoint: '/api/registry/click',
      generated: '產生時間',
      items: '項',
      label: '註冊表指標',
      loadError: '無法載入指標',
      perCategory: '依分類統計',
      title: '點擊遙測',
      topItems: '整體熱門項目',
      totalClicks: '總點擊數',
      uniqueItems: '不重複項目數',
    },
    deploy: {
      app: '應用程式',
      backToPlatforms: '返回平台選擇',
      badges: ['內建免費 LLM', '無需 API 金鑰', '1 GB 儲存空間'],
      copyrightSuffix: 'Agent 作業系統',
      deployFailed: '部署失敗',
      deployToFly: '部署至 Fly.io',
      deployed: '部署完成！',
      deployedDesc: '您的 LibreFang 執行個體正在啟動（約 1–2 分鐘）。免費 LLM（Step 3.5 Flash）已預先設定完成，可立即使用。',
      deploying: '部署中…',
      flyConsole: 'Fly.io 控制台',
      freeNote: '系統已預先設定好免費 LLM（透過 OpenRouter 提供的 Step 3.5 Flash），您的執行個體開箱即用，無需 API 金鑰。',
      includedModel: 'Step 3.5 Flash（免費）',
      installLocally: '本機安裝',
      localInstalls: {
        linux: {
          desc: '安裝腳本或下載執行檔',
          name: 'Linux',
        },
        macos: {
          desc: '使用 Homebrew 或下載執行檔',
          name: 'macOS',
        },
        windows: {
          desc: 'PowerShell 安裝程式或 .msi',
          name: 'Windows',
        },
      },
      logIn: '登入',
      model: '模型',
      openDashboard: '開啟儀表板',
      or: '或',
      personalAccessTokens: 'Personal Access Tokens',
      platforms: {
        docker: {
          desc: '一個指令，隨處執行',
        },
        flyio: {
          badge: '推薦',
          demo: '線上示範',
          desc: '永久免費，具持久性儲存',
        },
        gcp: {
          badge: 'Terraform',
          desc: '永久免費（e2-micro），30GB 儲存空間',
        },
        railway: {
          desc: '部署簡單，贈送 5 美元免費額度',
        },
        render: {
          badge: '最簡單',
          demo: '線上示範',
          desc: '一鍵 OAuth 部署',
          warning: '免費方案：閒置 15 分鐘後會進入休眠，且無持久性儲存',
        },
      },
      region: '地區',
      signUp: '註冊',
      stepOneMiddle: 'Fly.io 帳戶，然後前往',
      stepOnePrefix: '',
      stepOneSuffix: '並建立一個新權杖。',
      stepOneTitle: '取得 Fly.io API 權杖',
      stepTwoDesc: '您的權杖僅會傳送至 Fly.io API，絕不會儲存在我們的伺服器上。',
      stepTwoTitle: '貼上並部署',
      steps: {
        app: '正在建立應用程式…',
        auth: '正在驗證權杖…',
        machine: '正在以 Step 3.5 Flash 啟動機器…',
        net: '正在配置 IP 位址…',
        vol: '正在建立持久性磁碟區…',
      },
      subtitle: '選擇您的平台',
      terminalIntro: '或從終端機部署：',
      title: '部署 LibreFang',
      tokenLabel: 'Fly.io API 權杖',
      tokenRequired: '請輸入您的 Fly.io API 權杖。',
      troubleshooting: '疑難排解',
      troubleshootingItems: {
        image: {
          lines: [
            [
              {
                text: 'Docker 映像檔 ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' 會在每次發行版本時建置。',
              },
            ],
            [
              {
                text: '若尚未發布任何版本，請使用下方的終端機部署腳本，它會從原始碼建置。',
              },
            ],
          ],
          q: '部署失敗：找不到映像檔',
        },
        llm: {
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: '然後透過 ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ' 編輯 ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' 以更新預設模型。',
              },
            ],
          ],
          q: '部署後要如何新增或變更 LLM 供應商？',
        },
        sso: {
          lines: [
            [
              {
                text: '若您看到：',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: '請改用組織專屬權杖，於終端機執行：',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: '然後將產生的權杖貼到上方欄位。',
              },
            ],
          ],
          q: '無法建立 Personal Access Token（SSO 錯誤）',
        },
      },
      upgradeModel: '升級模型',
      website: '網站',
    },
    changelog: {
      backHome: '回到首頁',
      changeTypes: {
        breaking: '重大變更',
        feature: '新增',
        fix: '修復',
        other: '其他',
        performance: '效能',
      },
      desc: '追蹤 LibreFang 的每一次更新',
      downloads: '次下載',
      empty: '找不到任何版本。',
      filters: {
        all: '全部',
        prerelease: '預發布版',
        stable: '穩定版',
      },
      fullNotes: '完整發行說明請見 GitHub。',
      loadError: '載入版本失敗',
      loading: '正在載入版本…',
      noFilterMatches: '沒有符合目前篩選條件的版本。',
      releaseTypes: {
        beta: 'Beta',
        rc: 'RC',
        stable: '穩定版',
      },
      releases: '個版本',
      title: '更新日誌',
      unknownError: '未知錯誤',
      viewOnGitHub: '在 GitHub 上查看',
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
        '16層のセキュリティで監視',
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
      kernelDescs: ['作成、開始、一時停止、再開、停止、破棄', '9つの組み込みテンプレート、DAGオーケストレーション', 'エージェント別支出制限、グローバル上限、アラート', 'Cronベースのトリガー、定期タスク、イベントフック', '短期、長期、エピソード、セマンティック、オートドリーム統合', 'Python & Promptスキル、ホットリロード', 'Model Context Protocol、エージェント間通信', 'Open Fang Protocol、メッシュネットワーク'],
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
      downloads: 'ダウンロード',
      stars: 'スター',
      thisWeek: '今週',
      sort: { label: '並び替え', popular: '人気順', nameAsc: '名前 A–Z', nameDesc: '名前 Z–A', trending: 'クリック数', downloads: 'ダウンロード数順', weekly: '今週のトレンド' },
      onThisPage: 'このページ',
      previous: '前へ',
      next: '次へ',
      prevNext: 'カテゴリ内の前後',
      readme: 'README',
      viewHistory: '履歴',
      templateDiff: 'テンプレート差分',
      subcategories: {
        ai: 'AI', business: 'ビジネス', cloud: 'クラウド', communication: 'コミュニケーション',
        content: 'コンテンツ', creation: '制作', data: 'データ', developer: '開発者',
        development: '開発', devtools: '開発ツール', email: 'メール',
        engineering: 'エンジニアリング', enterprise: 'エンタープライズ', iot: 'IoT',
        language: '言語', messaging: 'メッセージング', productivity: '生産性',
        research: 'リサーチ', skills: 'スキル', social: 'ソーシャル', thinking: '思考',
      },
      categories: {
        skills:   { title: 'スキル', desc: 'プラグ可能なツールバンドル —— Python、WASM、Node、または prompt-only スキルで Agent の能力を拡張。' },
        mcp:      { title: 'MCP サーバー', desc: 'Model Context Protocol サーバーで、外部ツールとデータソースを任意の Agent に直接接続。' },
        plugins:  { title: 'プラグイン', desc: 'LibreFang デーモンにカスタムコマンド・チャネル・挙動を追加するランタイム拡張。' },
        hands:    { title: 'Hands', desc: '自律的な能力ユニット。各 Hand はモデル、ツール、ワークフローを同梱 —— 組み立てずに有効化。' },
        agents:   { title: 'Agent', desc: 'プリビルトの Agent テンプレート。モデル・システムプロンプト・能力・スケジュールを1つのマニフェストに。' },
        providers:{ title: 'プロバイダー', desc: 'LLM プロバイダーアダプター：Anthropic、OpenAI、Gemini、Groq、ローカル、その他 40+。' },
        workflows:{ title: 'ワークフロー', desc: 'TOML で書かれた多段 Agent オーケストレーション。Agent の連結、条件分岐、状態の永続化。' },
        channels: { title: 'チャネル', desc: 'メッセージングアダプター：Telegram、Slack、Discord、WhatsApp、LINE など 44 プラットフォーム。' },
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
    common: {
      agentOs: 'Agent OS',
      backToTop: 'トップに戻る',
      breadcrumb: 'ブレッドクラム',
      contributors: 'コントリビューター',
      copied: 'コピーしました！',
      copy: 'コピー',
      copyToClipboard: 'クリップボードにコピー',
      loadingReleases: 'リリースを読み込み中...',
      skipToContent: 'コンテンツへスキップ',
      viewFull: 'すべて表示',
    },
    header: {
      source: 'ソース',
      switchLanguage: '言語を切り替え',
      switchLanguageNamed: '言語を切り替え（{name}）',
      toggleMenu: 'メニューを切り替え',
      toggleTheme: 'テーマを切り替え',
      viewSource: 'GitHub でソースを見る',
    },
    errorBoundary: {
      home: 'ホーム',
      label: '実行時エラー',
      reload: '再読み込み',
      title: '問題が発生しました。',
    },
    metrics: {
      activeCategories: '有効なカテゴリ数',
      breadcrumb: 'メトリクス',
      clicks: 'クリック',
      desc: 'レジストリ全体の詳細ページビューの集計数。/api/registry/click ワーカーエンドポイント経由で記録されます。',
      descPrefix: 'レジストリ全体の詳細ページビューの集計数。',
      descSuffix: ' ワーカーエンドポイント経由で記録されます。',
      endpoint: '/api/registry/click',
      generated: '生成日時',
      items: '件',
      label: 'レジストリメトリクス',
      loadError: 'メトリクスを読み込めませんでした',
      perCategory: 'カテゴリ別',
      title: 'クリックテレメトリー',
      topItems: '全体のトップアイテム',
      totalClicks: '総クリック数',
      uniqueItems: 'ユニークアイテム数',
    },
    deploy: {
      app: 'アプリ',
      backToPlatforms: 'プラットフォーム一覧に戻る',
      badges: ['無料 LLM 付属', 'API キー不要', '1 GB ストレージ'],
      copyrightSuffix: 'Agent Operating System',
      deployFailed: 'デプロイに失敗しました',
      deployToFly: 'Fly.io にデプロイ',
      deployed: 'デプロイ完了！',
      deployedDesc: 'LibreFang インスタンスが起動中です（1〜2分）。無料 LLM（Step 3.5 Flash）は事前設定済みで、そのまま利用できます。',
      deploying: 'デプロイ中...',
      flyConsole: 'Fly.io コンソール',
      freeNote: '無料 LLM（OpenRouter 経由の Step 3.5 Flash）が事前設定されています。API キーなしで、そのまますぐに使えます。',
      includedModel: 'Step 3.5 Flash（無料）',
      installLocally: 'ローカルにインストール',
      localInstalls: {
        linux: {
          desc: 'インストールスクリプトまたはバイナリをダウンロード',
          name: 'Linux',
        },
        macos: {
          desc: 'Homebrew またはバイナリをダウンロード',
          name: 'macOS',
        },
        windows: {
          desc: 'PowerShell インストーラーまたは .msi',
          name: 'Windows',
        },
      },
      logIn: 'ログイン',
      model: 'モデル',
      openDashboard: 'ダッシュボードを開く',
      or: 'または',
      personalAccessTokens: 'Personal Access Tokens',
      platforms: {
        docker: {
          desc: 'コマンド1つでどこでも実行',
        },
        flyio: {
          badge: 'おすすめ',
          demo: 'ライブデモ',
          desc: '永久無料、永続ストレージ',
        },
        gcp: {
          badge: 'Terraform',
          desc: '永久無料（e2-micro）、30GB ストレージ',
        },
        railway: {
          desc: '$5 分の無料クレジットで簡単にデプロイ',
        },
        render: {
          badge: '最も簡単',
          demo: 'ライブデモ',
          desc: 'OAuth によるワンクリックデプロイ',
          warning: '無料プラン：15分後にスリープ、永続ストレージなし',
        },
      },
      region: 'リージョン',
      signUp: 'サインアップ',
      stepOneMiddle: 'して Fly.io にアクセスし、',
      stepOnePrefix: '',
      stepOneSuffix: 'に移動して新しいトークンを作成してください。',
      stepOneTitle: 'Fly.io の API トークンを取得',
      stepTwoDesc: 'トークンは Fly.io の API にのみ送信され、当サービスのサーバーには保存されません。',
      stepTwoTitle: '貼り付けてデプロイ',
      steps: {
        app: 'アプリを作成中...',
        auth: 'トークンを確認中...',
        machine: 'Step 3.5 Flash でマシンを起動中...',
        net: 'IP アドレスを割り当て中...',
        vol: '永続ボリュームを作成中...',
      },
      subtitle: 'プラットフォームを選択してください',
      terminalIntro: 'またはターミナルからデプロイ：',
      title: 'LibreFang をデプロイ',
      tokenLabel: 'Fly.io API トークン',
      tokenRequired: 'Fly.io API トークンを入力してください。',
      troubleshooting: 'トラブルシューティング',
      troubleshootingItems: {
        image: {
          lines: [
            [
              {
                text: 'Docker イメージ ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' は各リリースごとにビルドされます。',
              },
            ],
            [
              {
                text: 'まだリリースが公開されていない場合は、下記のターミナルデプロイスクリプトを使用してください。ソースからビルドされます。',
              },
            ],
          ],
          q: 'デプロイ失敗：イメージが見つかりません',
        },
        llm: {
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: '次に ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ' 経由で ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' を編集し、デフォルトモデルを更新してください。',
              },
            ],
          ],
          q: 'デプロイ後に LLM プロバイダーを追加・変更するには？',
        },
        sso: {
          lines: [
            [
              {
                text: '次のように表示される場合：',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: '代わりに組織単位のトークンを使用してください。ターミナルで次を実行：',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: '生成されたトークンを上記の欄に貼り付けてください。',
              },
            ],
          ],
          q: 'Personal Access Token を作成できない（SSO エラー）',
        },
      },
      upgradeModel: 'モデルをアップグレード',
      website: 'ウェブサイト',
    },
    changelog: {
      backHome: 'ホームに戻る',
      changeTypes: {
        breaking: '破壊的変更',
        feature: '追加',
        fix: '修正',
        other: 'その他',
        performance: 'パフォーマンス',
      },
      desc: 'LibreFang のすべての更新を追跡',
      downloads: 'ダウンロード数',
      empty: 'リリースが見つかりません。',
      filters: {
        all: 'すべて',
        prerelease: 'プレリリース',
        stable: '安定版',
      },
      fullNotes: 'GitHub で完全なリリースノートを確認してください。',
      loadError: 'リリースの読み込みに失敗しました',
      loading: 'リリースを読み込み中...',
      noFilterMatches: '現在のフィルターに一致するリリースはありません。',
      releaseTypes: {
        beta: 'ベータ',
        rc: 'RC',
        stable: '安定版',
      },
      releases: 'リリース',
      title: '変更履歴',
      unknownError: '不明なエラー',
      viewOnGitHub: 'GitHub で見る',
    },
    footer: { docs: 'ドキュメント', license: 'ライセンス', privacy: 'プライバシー', changelog: '変更履歴' },
  },

  ko: {
    nav: { architecture: '아키텍처', hands: 'Hands', performance: '성능', install: '설치', downloads: '다운로드', docs: '문서', features: '마켓플레이스', evolution: '스킬 자가 진화', workflows: '워크플로우', registry: '레지스트리', learnMore: '기능' },
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
      kernelDescs: ['생성, 시작, 일시정지, 재개, 중지, 삭제', '9개 내장 템플릿, DAG 오케스트레이션', '에이전트별 지출 한도, 전역 상한, 알림', 'Cron 기반 트리거, 주기 작업, 이벤트 훅', '단기, 장기, 에피소드, 시맨틱, 오토드림 통합', 'Python & Prompt 스킬, 핫 리로드', 'Model Context Protocol, 에이전트 간 통신', 'Open Fang Protocol, 메시 네트워킹'],
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
      downloads: '다운로드',
      stars: '스타',
      thisWeek: '이번 주',
      sort: { label: '정렬', popular: '인기순', nameAsc: '이름 A–Z', nameDesc: '이름 Z–A', trending: '클릭수', downloads: '다운로드 많은 순', weekly: '이번 주 인기' },
      onThisPage: '이 페이지',
      previous: '이전',
      next: '다음',
      prevNext: '카테고리 내 이전 / 다음',
      readme: 'README',
      viewHistory: '기록',
      templateDiff: '템플릿 차이',
      subcategories: {
        ai: 'AI', business: '비즈니스', cloud: '클라우드', communication: '커뮤니케이션',
        content: '콘텐츠', creation: '제작', data: '데이터', developer: '개발자',
        development: '개발', devtools: '개발 도구', email: '이메일',
        engineering: '엔지니어링', enterprise: '엔터프라이즈', iot: 'IoT',
        language: '언어', messaging: '메시징', productivity: '생산성',
        research: '리서치', skills: '스킬', social: '소셜', thinking: '사고',
      },
      categories: {
        skills:   { title: '스킬', desc: '플러그 가능한 도구 번들 —— Python, WASM, Node 또는 prompt-only 스킬로 Agent의 능력 확장.' },
        mcp:      { title: 'MCP 서버', desc: 'Model Context Protocol 서버로 외부 도구와 데이터 소스를 모든 Agent에 직접 연결.' },
        plugins:  { title: '플러그인', desc: 'LibreFang 데몬에 커스텀 명령어, 채널, 동작을 추가하는 런타임 확장.' },
        hands:    { title: 'Hands', desc: '자율 능력 유닛. 각 Hand는 자체 모델, 도구, 워크플로를 포함 —— 조립 없이 활성화.' },
        agents:   { title: 'Agent 템플릿', desc: '사전 구축된 Agent 템플릿. 모델, 시스템 프롬프트, 권한, 스케줄을 하나의 매니페스트에.' },
        providers:{ title: '제공자', desc: 'LLM 제공자 어댑터: Anthropic, OpenAI, Gemini, Groq, 로컬 등 40개 이상.' },
        workflows:{ title: '워크플로우', desc: 'TOML로 작성된 다단계 Agent 오케스트레이션. Agent 연결, 조건 분기, 상태 지속성.' },
        channels: { title: '채널', desc: '메시징 어댑터: Telegram, Slack, Discord, WhatsApp, LINE 등 44개 플랫폼.' },
      },
    },
    search: {
      title: '레지스트리 검색',
      placeholder: '스킬, Hand, Agent, 제공자 검색…',
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
    common: {
      copy: '복사',
      copied: '복사됨!',
      copyToClipboard: '클립보드에 복사',
      loadingReleases: '릴리스를 불러오는 중...',
      skipToContent: '본문으로 건너뛰기',
      backToTop: '맨 위로',
      viewFull: '전체 보기',
      contributors: '기여자',
      agentOs: 'Agent OS',
      breadcrumb: '이동 경로',
    },
    header: {
      toggleTheme: '테마 전환',
      viewSource: 'GitHub에서 소스 보기',
      source: '소스',
      switchLanguage: '언어 전환',
      switchLanguageNamed: '언어 전환 ({name})',
      toggleMenu: '메뉴 전환',
    },
    errorBoundary: {
      label: '런타임 오류',
      title: '문제가 발생했습니다.',
      reload: '새로고침',
      home: '홈',
    },
    metrics: {
      breadcrumb: '메트릭',
      label: '레지스트리 메트릭',
      title: '클릭 텔레메트리',
      desc: '레지스트리 전체의 상세 페이지 조회수 집계이며, /api/registry/click 워커 엔드포인트를 통해 기록됩니다.',
      descPrefix: '레지스트리 전체의 상세 페이지 조회수 집계이며, ',
      endpoint: '/api/registry/click',
      descSuffix: ' 워커 엔드포인트를 통해 기록됩니다.',
      loadError: '메트릭을 불러올 수 없습니다',
      totalClicks: '총 클릭 수',
      uniqueItems: '고유 항목 수',
      activeCategories: '활성 카테고리',
      generated: '생성 시각',
      perCategory: '카테고리별',
      topItems: '전체 상위 항목',
      items: '개 항목',
      clicks: '클릭',
    },
    deploy: {
      title: 'LibreFang 배포하기',
      subtitle: '플랫폼을 선택하세요',
      terminalIntro: '또는 터미널에서 직접 배포하세요:',
      installLocally: '로컬에 설치',
      website: '웹사이트',
      copyrightSuffix: 'Agent Operating System',
      backToPlatforms: '플랫폼 목록으로 돌아가기',
      badges: ['무료 LLM 포함', 'API 키 불필요', '1GB 스토리지'],
      tokenRequired: 'Fly.io API 토큰을 입력해 주세요.',
      deployFailed: '배포에 실패했습니다',
      deploying: '배포 중...',
      deployToFly: 'Fly.io에 배포',
      deployed: '배포 완료!',
      deployedDesc: 'LibreFang 인스턴스가 시작되고 있습니다 (1~2분 소요). 무료 LLM(Step 3.5 Flash)이 사전 설정되어 바로 사용할 수 있습니다.',
      openDashboard: '대시보드 열기',
      flyConsole: 'Fly.io 콘솔',
      app: '앱',
      region: '리전',
      model: '모델',
      includedModel: 'Step 3.5 Flash (무료)',
      upgradeModel: '모델 업그레이드',
      freeNote: '무료 LLM(OpenRouter를 통한 Step 3.5 Flash)이 사전 설정되어 있습니다. API 키 없이도 인스턴스가 바로 작동합니다.',
      stepOneTitle: 'Fly.io API 토큰 발급받기',
      stepOnePrefix: '',
      signUp: 'Fly.io에 가입',
      or: '또는',
      logIn: '로그인',
      stepOneMiddle: '한 뒤',
      personalAccessTokens: 'Personal Access Tokens',
      stepOneSuffix: '페이지로 이동해 새 토큰을 생성하세요.',
      stepTwoTitle: '붙여넣고 배포하기',
      stepTwoDesc: '입력한 토큰은 Fly.io API로만 전송되며, 저희 서버에는 저장되지 않습니다.',
      tokenLabel: 'Fly.io API 토큰',
      troubleshooting: '문제 해결',
      platforms: {
        flyio: {
          desc: '영구 무료, 영속 스토리지 제공',
          badge: '추천',
          demo: '라이브 데모',
        },
        render: {
          desc: '원클릭 OAuth 배포',
          badge: '가장 쉬움',
          demo: '라이브 데모',
          warning: '무료 티어: 15분 후 슬립 모드, 영속 스토리지 없음',
        },
        railway: {
          desc: '5달러 무료 크레딧으로 간단하게 배포',
        },
        gcp: {
          desc: '영구 무료(e2-micro), 30GB 스토리지',
          badge: 'Terraform',
        },
        docker: {
          desc: '한 줄 명령으로 어디서든 실행',
        },
      },
      localInstalls: {
        macos: {
          name: 'macOS',
          desc: 'Homebrew 또는 바이너리 다운로드',
        },
        linux: {
          name: 'Linux',
          desc: '설치 스크립트 또는 바이너리 다운로드',
        },
        windows: {
          name: 'Windows',
          desc: 'PowerShell 설치 프로그램 또는 .msi',
        },
      },
      steps: {
        auth: '토큰 확인 중...',
        app: '앱 생성 중...',
        net: 'IP 주소 할당 중...',
        vol: '영속 볼륨 생성 중...',
        machine: 'Step 3.5 Flash로 머신 시작 중...',
      },
      troubleshootingItems: {
        sso: {
          q: 'Personal Access Token을 생성할 수 없음 (SSO 오류)',
          lines: [
            [
              {
                text: '다음과 같은 오류가 표시된다면: ',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: '대신 조직별 토큰을 사용하세요. 터미널에서 다음을 실행하세요:',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: '생성된 토큰을 위 입력란에 붙여넣으세요.',
              },
            ],
          ],
        },
        image: {
          q: '배포 실패: 이미지를 찾을 수 없음',
          lines: [
            [
              {
                text: 'Docker 이미지 ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: '는 릴리스마다 새로 빌드됩니다.',
              },
            ],
            [
              {
                text: '아직 게시된 릴리스가 없다면 아래의 터미널 배포 스크립트를 사용하세요 — 소스에서 직접 빌드합니다.',
              },
            ],
          ],
        },
        llm: {
          q: '배포 후 LLM 제공자를 추가하거나 변경하는 방법은?',
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: '그런 다음 ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: '로 ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: '을 열어 기본 모델을 업데이트하세요.',
              },
            ],
          ],
        },
      },
    },
    changelog: {
      title: '변경 이력',
      desc: 'LibreFang의 모든 업데이트를 확인하세요',
      backHome: '홈으로 돌아가기',
      releases: '릴리스',
      downloads: '다운로드',
      loading: '릴리스를 불러오는 중...',
      loadError: '릴리스를 불러오지 못했습니다',
      unknownError: '알 수 없는 오류',
      empty: '릴리스가 없습니다.',
      noFilterMatches: '현재 필터와 일치하는 릴리스가 없습니다.',
      fullNotes: 'GitHub에서 전체 릴리스 노트를 확인하세요.',
      viewOnGitHub: 'GitHub에서 보기',
      filters: {
        all: '전체',
        stable: '안정 버전',
        prerelease: '사전 릴리스',
      },
      releaseTypes: {
        stable: '안정 버전',
        rc: 'RC',
        beta: '베타',
      },
      changeTypes: {
        feature: '추가됨',
        fix: '수정됨',
        breaking: '주요 변경',
        performance: '성능',
        other: '기타',
      },
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
        'mit 16 Sicherheitsschichten überwachen',
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
      kernelDescs: ['Erstellen, starten, pausieren, fortsetzen, stoppen, zerstören', '9 eingebaute Vorlagen, DAG-Orchestrierung', 'Ausgabelimits pro Agent, globale Obergrenzen, Warnungen', 'Cron-basierte Trigger, Intervall-Tasks, Event-Hooks', 'Kurzzeit, Langzeit, episodisch, semantisch, Auto-Dream-Konsolidierung', 'Python & Prompt Skills, Hot-Reload', 'Model Context Protocol, Agent-to-Agent', 'Open Fang Protocol für Mesh-Netzwerk'],
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
        { title: 'Automatische Erkennung', desc: '5+ Tool-Aufrufe lösen eine Hintergrundüberprüfung des Ansatzes aus.' },
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
      downloads: 'Downloads',
      stars: 'Sterne',
      thisWeek: 'diese Woche',
      sort: { label: 'Sortieren', popular: 'Beliebt', nameAsc: 'Name A–Z', nameDesc: 'Name Z–A', trending: 'Meistgeklickt', downloads: 'Meistgeladen', weekly: 'Trends diese Woche' },
      onThisPage: 'Auf dieser Seite',
      previous: 'Zurück',
      next: 'Weiter',
      prevNext: 'Vorheriges / nächstes in der Kategorie',
      readme: 'README',
      viewHistory: 'Verlauf',
      templateDiff: 'Template-Diff',
      subcategories: {
        ai: 'KI', business: 'Business', cloud: 'Cloud', communication: 'Kommunikation',
        content: 'Inhalt', creation: 'Erstellung', data: 'Daten', developer: 'Entwickler',
        development: 'Entwicklung', devtools: 'DevTools', email: 'E-Mail',
        engineering: 'Engineering', enterprise: 'Enterprise', iot: 'IoT',
        language: 'Sprache', messaging: 'Messaging', productivity: 'Produktivität',
        research: 'Forschung', skills: 'Skills', social: 'Social', thinking: 'Denken',
      },
      categories: {
        skills:   { title: 'Skills', desc: 'Austauschbare Tool-Bundles — Python-, WASM-, Node- oder Prompt-Only-Skills, die die Fähigkeiten eines Agenten erweitern.' },
        mcp:      { title: 'MCP-Server', desc: 'Model-Context-Protocol-Server, die externe Tools und Datenquellen direkt in jeden Agenten einbinden.' },
        plugins:  { title: 'Plugins', desc: 'Runtime-Erweiterungen, die dem LibreFang-Daemon benutzerdefinierte Befehle, Kanäle oder Verhalten hinzufügen.' },
        hands:    { title: 'Hands', desc: 'Autonome Fähigkeitseinheiten. Jede Hand bringt ihr eigenes Modell, Tools und Workflow mit — aktivieren statt zusammenbauen.' },
        agents:   { title: 'Agenten', desc: 'Vorgefertigte Agent-Vorlagen. Modell, System-Prompt, Berechtigungen und Zeitplan in einem Manifest.' },
        providers:{ title: 'Anbieter', desc: 'LLM-Anbieter-Adapter: Anthropic, OpenAI, Gemini, Groq, lokal — und 40+ weitere.' },
        workflows:{ title: 'Workflows', desc: 'Mehrstufige Agent-Orchestrierungen in TOML. Agenten verketten, auf Bedingungen verzweigen, Zustand persistieren.' },
        channels: { title: 'Kanäle', desc: 'Messaging-Adapter: Telegram, Slack, Discord, WhatsApp, LINE und 40+ weitere Plattformen.' },
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
      desc: 'Alle Kategorien auf einen Blick — wähle eine für die vollständige Liste, sortiert nach Beliebtheit.',
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
    common: {
      copy: 'Kopieren',
      copied: 'Kopiert!',
      copyToClipboard: 'In die Zwischenablage kopieren',
      loadingReleases: 'Releases werden geladen...',
      skipToContent: 'Zum Inhalt springen',
      backToTop: 'Nach oben',
      viewFull: 'Vollständig anzeigen',
      contributors: 'Mitwirkende',
      agentOs: 'Agent OS',
      breadcrumb: 'Breadcrumb',
    },
    header: {
      toggleTheme: 'Design umschalten',
      viewSource: 'Quellcode auf GitHub ansehen',
      source: 'Quelle',
      switchLanguage: 'Sprache wechseln',
      switchLanguageNamed: 'Sprache wechseln ({name})',
      toggleMenu: 'Menü umschalten',
    },
    errorBoundary: {
      label: 'Laufzeitfehler',
      title: 'Etwas ist schiefgelaufen.',
      reload: 'Neu laden',
      home: 'Startseite',
    },
    metrics: {
      breadcrumb: 'Metriken',
      label: 'Registry-Metriken',
      title: 'Klick-Telemetrie',
      desc: 'Aggregierte Zählung der Detailseiten-Aufrufe in der Registry, erfasst über den /api/registry/click Worker-Endpunkt.',
      descPrefix: 'Aggregierte Zählung der Detailseiten-Aufrufe in der Registry, erfasst über den ',
      endpoint: '/api/registry/click',
      descSuffix: ' Worker-Endpunkt.',
      loadError: 'Metriken konnten nicht geladen werden',
      totalClicks: 'Klicks insgesamt',
      uniqueItems: 'Eindeutige Einträge',
      activeCategories: 'Aktive Kategorien',
      generated: 'Erstellt',
      perCategory: 'Nach Kategorie',
      topItems: 'Top-Einträge insgesamt',
      items: 'Einträge',
      clicks: 'Klicks',
    },
    deploy: {
      title: 'LibreFang bereitstellen',
      subtitle: 'Wähle deine Plattform',
      terminalIntro: 'Oder direkt aus dem Terminal bereitstellen:',
      installLocally: 'Lokal installieren',
      website: 'Website',
      copyrightSuffix: 'Agent Operating System',
      backToPlatforms: 'Zurück zu den Plattformen',
      badges: ['Kostenloses LLM inklusive', 'Kein API-Schlüssel nötig', '1 GB Speicher'],
      tokenRequired: 'Bitte gib deinen Fly.io API-Token ein.',
      deployFailed: 'Bereitstellung fehlgeschlagen',
      deploying: 'Wird bereitgestellt...',
      deployToFly: 'Auf Fly.io bereitstellen',
      deployed: 'Bereitgestellt!',
      deployedDesc: 'Deine LibreFang-Instanz startet gerade (1–2 Min.). Das kostenlose LLM (Step 3.5 Flash) ist bereits vorkonfiguriert und einsatzbereit.',
      openDashboard: 'Dashboard öffnen',
      flyConsole: 'Fly.io-Konsole',
      app: 'App',
      region: 'Region',
      model: 'Modell',
      includedModel: 'Step 3.5 Flash (kostenlos)',
      upgradeModel: 'Modell upgraden',
      freeNote: 'Ein kostenloses LLM (Step 3.5 Flash über OpenRouter) ist bereits vorkonfiguriert. Deine Instanz funktioniert sofort — kein API-Schlüssel erforderlich.',
      stepOneTitle: 'Fly.io API-Token besorgen',
      stepOnePrefix: '',
      signUp: 'Registriere dich',
      or: 'oder',
      logIn: 'melde dich an',
      stepOneMiddle: 'bei Fly.io, gehe dann zu',
      personalAccessTokens: 'Personal Access Tokens',
      stepOneSuffix: 'und erstelle einen neuen Token.',
      stepTwoTitle: 'Einfügen und bereitstellen',
      stepTwoDesc: 'Dein Token wird ausschließlich an die Fly.io-API gesendet und niemals auf unseren Servern gespeichert.',
      tokenLabel: 'Fly.io API-Token',
      troubleshooting: 'Fehlerbehebung',
      platforms: {
        flyio: {
          desc: 'Dauerhaft kostenlos, persistenter Speicher',
          badge: 'Empfohlen',
          demo: 'Live-Demo',
        },
        render: {
          desc: 'Bereitstellung per OAuth mit einem Klick',
          badge: 'Am einfachsten',
          demo: 'Live-Demo',
          warning: 'Kostenlose Stufe: schläft nach 15 Min., kein persistenter Speicher',
        },
        railway: {
          desc: 'Einfache Bereitstellung mit 5 $ Startguthaben',
        },
        gcp: {
          desc: 'Dauerhaft kostenlos (e2-micro), 30 GB Speicher',
          badge: 'Terraform',
        },
        docker: {
          desc: 'Ein Befehl, läuft überall',
        },
      },
      localInstalls: {
        macos: {
          name: 'macOS',
          desc: 'Homebrew oder Binary herunterladen',
        },
        linux: {
          name: 'Linux',
          desc: 'Installationsskript oder Binary herunterladen',
        },
        windows: {
          name: 'Windows',
          desc: 'PowerShell-Installer oder .msi',
        },
      },
      steps: {
        auth: 'Token wird überprüft...',
        app: 'App wird erstellt...',
        net: 'IP-Adressen werden zugewiesen...',
        vol: 'Persistentes Volume wird erstellt...',
        machine: 'Maschine mit Step 3.5 Flash wird gestartet...',
      },
      troubleshootingItems: {
        sso: {
          q: 'Personal Access Token kann nicht erstellt werden (SSO-Fehler)',
          lines: [
            [
              {
                text: 'Wenn folgende Meldung erscheint: ',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: 'Verwende stattdessen einen organisationsspezifischen Token. Führe im Terminal aus:',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: 'Füge dann den generierten Token oben ein.',
              },
            ],
          ],
        },
        image: {
          q: 'Bereitstellung fehlgeschlagen: Image nicht gefunden',
          lines: [
            [
              {
                text: 'Das Docker-Image ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' wird bei jedem Release neu erstellt.',
              },
            ],
            [
              {
                text: 'Falls noch kein Release veröffentlicht wurde, verwende das Terminal-Deploy-Skript unten — es baut direkt aus dem Quellcode.',
              },
            ],
          ],
        },
        llm: {
          q: 'Wie kann ich nach der Bereitstellung einen LLM-Anbieter hinzufügen oder wechseln?',
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: 'Bearbeite anschließend ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' über ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ', um das Standardmodell zu aktualisieren.',
              },
            ],
          ],
        },
      },
    },
    changelog: {
      title: 'Changelog',
      desc: 'Verfolge jedes Update von LibreFang',
      backHome: 'Zurück zur Startseite',
      releases: 'Releases',
      downloads: 'Downloads',
      loading: 'Releases werden geladen...',
      loadError: 'Releases konnten nicht geladen werden',
      unknownError: 'Unbekannter Fehler',
      empty: 'Keine Releases gefunden.',
      noFilterMatches: 'Keine Releases entsprechen dem aktuellen Filter.',
      fullNotes: 'Vollständige Release-Notes auf GitHub ansehen.',
      viewOnGitHub: 'Auf GitHub ansehen',
      filters: {
        all: 'Alle',
        stable: 'Stabil',
        prerelease: 'Vorabversion',
      },
      releaseTypes: {
        stable: 'Stabil',
        rc: 'RC',
        beta: 'Beta',
      },
      changeTypes: {
        feature: 'Hinzugefügt',
        fix: 'Behoben',
        breaking: 'Breaking',
        performance: 'Performance',
        other: 'Sonstiges',
      },
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
      kernelDescs: ['Crear, iniciar, pausar, reanudar, detener, destruir', '9 plantillas integradas, orquestación DAG', 'Límites de gasto por agente, topes globales, alertas', 'Triggers basados en Cron, tareas de intervalo, hooks de eventos', 'Corto plazo, largo plazo, episódica, semántica, consolidación auto-dream', 'Skills Python y Prompt, recarga en caliente', 'Model Context Protocol, Agent-to-Agent', 'Open Fang Protocol para redes mesh'],
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
      tagline: 'Autónomo · Versionado · Escaneado por seguridad',
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
      downloads: 'descargas',
      stars: 'estrellas',
      thisWeek: 'esta semana',
      sort: { label: 'Ordenar', popular: 'Populares', nameAsc: 'Nombre A–Z', nameDesc: 'Nombre Z–A', trending: 'Más clics', downloads: 'Más descargados', weekly: 'Tendencia esta semana' },
      onThisPage: 'En esta página',
      previous: 'Anterior',
      next: 'Siguiente',
      prevNext: 'Anterior / siguiente en la categoría',
      readme: 'README',
      viewHistory: 'Historial',
      templateDiff: 'Diferencias de plantilla',
      subcategories: {
        ai: 'IA', business: 'Negocio', cloud: 'Nube', communication: 'Comunicación',
        content: 'Contenido', creation: 'Creación', data: 'Datos', developer: 'Desarrollador',
        development: 'Desarrollo', devtools: 'DevTools', email: 'Correo',
        engineering: 'Ingeniería', enterprise: 'Empresa', iot: 'IoT',
        language: 'Lenguaje', messaging: 'Mensajería', productivity: 'Productividad',
        research: 'Investigación', skills: 'Skills', social: 'Social', thinking: 'Pensamiento',
      },
      categories: {
        skills:   { title: 'Skills', desc: 'Paquetes de herramientas conectables — skills Python, WASM, Node o prompt-only que amplían las capacidades del agente.' },
        mcp:      { title: 'Servidores MCP', desc: 'Servidores Model Context Protocol que conectan herramientas y fuentes de datos externas directamente a cualquier agente.' },
        plugins:  { title: 'Plugins', desc: 'Extensiones en runtime que añaden comandos, canales o comportamientos al daemon LibreFang.' },
        hands:    { title: 'Hands', desc: 'Unidades autónomas de capacidad. Cada Hand trae su propio modelo, herramientas y workflow — actívalo, no lo montes.' },
        agents:   { title: 'Agentes', desc: 'Plantillas de agente listas. Modelo, prompt del sistema, capacidades y programación en un solo manifiesto.' },
        providers:{ title: 'Proveedores', desc: 'Adaptadores de proveedores LLM: Anthropic, OpenAI, Gemini, Groq, local — y 40+ más.' },
        workflows:{ title: 'Workflows', desc: 'Orquestaciones multi-paso de agentes en TOML. Encadena agentes, bifurca por condiciones, persiste estado.' },
        channels: { title: 'Canales', desc: 'Adaptadores de mensajería: Telegram, Slack, Discord, WhatsApp, LINE y 40+ plataformas más.' },
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
      desc: 'Todas las categorías de un vistazo — elige una para la lista completa, ordenada por popularidad.',
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
    common: {
      agentOs: 'Agent OS',
      backToTop: 'Volver arriba',
      breadcrumb: 'Ruta de navegación',
      contributors: 'Colaboradores',
      copied: '¡Copiado!',
      copy: 'Copiar',
      copyToClipboard: 'Copiar al portapapeles',
      loadingReleases: 'Cargando versiones...',
      skipToContent: 'Saltar al contenido',
      viewFull: 'Ver completo',
    },
    header: {
      source: 'Código fuente',
      switchLanguage: 'Cambiar idioma',
      switchLanguageNamed: 'Cambiar idioma ({name})',
      toggleMenu: 'Alternar menú',
      toggleTheme: 'Cambiar tema',
      viewSource: 'Ver código fuente en GitHub',
    },
    errorBoundary: {
      home: 'Inicio',
      label: 'Error en tiempo de ejecución',
      reload: 'Recargar',
      title: 'Algo salió mal.',
    },
    metrics: {
      activeCategories: 'Categorías activas',
      breadcrumb: 'Métricas',
      clicks: 'clics',
      desc: 'Recuentos agregados de visitas a las páginas de detalle en todo el registry, registrados mediante el endpoint del worker /api/registry/click.',
      descPrefix: 'Recuentos agregados de visitas a las páginas de detalle en todo el registry, registrados mediante el endpoint del worker ',
      descSuffix: '.',
      endpoint: '/api/registry/click',
      generated: 'Generado',
      items: 'elementos',
      label: 'Métricas del Registry',
      loadError: 'No se pudieron cargar las métricas',
      perCategory: 'Por categoría',
      title: 'Telemetría de clics',
      topItems: 'Elementos más populares en total',
      totalClicks: 'Clics totales',
      uniqueItems: 'Elementos únicos',
    },
    deploy: {
      app: 'Aplicación',
      backToPlatforms: 'Volver a las plataformas',
      badges: ['LLM gratuito incluido', 'No se necesita clave de API', '1 GB de almacenamiento'],
      copyrightSuffix: 'Sistema Operativo de Agentes',
      deployFailed: 'El despliegue falló',
      deployToFly: 'Desplegar en Fly.io',
      deployed: '¡Desplegado!',
      deployedDesc: 'Tu instancia de LibreFang se está iniciando (1-2 min). El LLM gratuito (Step 3.5 Flash) ya está preconfigurado y listo para usar.',
      deploying: 'Desplegando...',
      flyConsole: 'Consola de Fly.io',
      freeNote: 'Hay un LLM gratuito (Step 3.5 Flash a través de OpenRouter) preconfigurado. Tu instancia funciona desde el primer momento, sin necesidad de claves de API.',
      includedModel: 'Step 3.5 Flash (gratis)',
      installLocally: 'Instalar localmente',
      localInstalls: {
        linux: {
          desc: 'Script de instalación o descarga del binario',
          name: 'Linux',
        },
        macos: {
          desc: 'Homebrew o descarga del binario',
          name: 'macOS',
        },
        windows: {
          desc: 'Instalador de PowerShell o .msi',
          name: 'Windows',
        },
      },
      logIn: 'inicia sesión',
      model: 'Modelo',
      openDashboard: 'Abrir panel',
      or: 'o',
      personalAccessTokens: 'Personal Access Tokens',
      platforms: {
        docker: {
          desc: 'Un solo comando, funciona en cualquier lugar',
        },
        flyio: {
          badge: 'Recomendado',
          demo: 'Demo en vivo',
          desc: 'Gratis para siempre, almacenamiento persistente',
        },
        gcp: {
          badge: 'Terraform',
          desc: 'Gratis para siempre (e2-micro), 30 GB de almacenamiento',
        },
        railway: {
          desc: 'Despliegue sencillo con $5 de crédito gratis',
        },
        render: {
          badge: 'Más fácil',
          demo: 'Demo en vivo',
          desc: 'Despliegue OAuth con un clic',
          warning: 'Plan gratuito: se suspende tras 15 min de inactividad, sin almacenamiento persistente',
        },
      },
      region: 'Región',
      signUp: 'Regístrate',
      stepOneMiddle: 'en Fly.io, luego ve a',
      stepOnePrefix: '',
      stepOneSuffix: 'y crea un nuevo token.',
      stepOneTitle: 'Obtén un token de API de Fly.io',
      stepTwoDesc: 'Tu token se envía únicamente a la API de Fly.io y nunca se almacena en nuestros servidores.',
      stepTwoTitle: 'Pega y despliega',
      steps: {
        app: 'Creando aplicación...',
        auth: 'Verificando token...',
        machine: 'Iniciando máquina con Step 3.5 Flash...',
        net: 'Asignando direcciones IP...',
        vol: 'Creando volumen persistente...',
      },
      subtitle: 'Elige tu plataforma',
      terminalIntro: 'O despliega desde tu terminal:',
      title: 'Despliega LibreFang',
      tokenLabel: 'Token de API de Fly.io',
      tokenRequired: 'Introduce tu token de API de Fly.io.',
      troubleshooting: 'Solución de problemas',
      troubleshootingItems: {
        image: {
          lines: [
            [
              {
                text: 'La imagen Docker ',
              },
              {
                code: 'ghcr.io/librefang/librefang:latest',
              },
              {
                text: ' se compila con cada versión.',
              },
            ],
            [
              {
                text: 'Si aún no se ha publicado ninguna versión, usa el script de despliegue por terminal de abajo: compila desde el código fuente.',
              },
            ],
          ],
          q: 'El despliegue falló: imagen no encontrada',
        },
        llm: {
          lines: [
            [
              {
                code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name',
              },
            ],
            [
              {
                text: 'Luego edita ',
              },
              {
                code: '/data/config.toml',
              },
              {
                text: ' mediante ',
              },
              {
                code: 'flyctl ssh console',
              },
              {
                text: ' para actualizar el modelo predeterminado.',
              },
            ],
          ],
          q: '¿Cómo añadir o cambiar el proveedor de LLM después del despliegue?',
        },
        sso: {
          lines: [
            [
              {
                text: 'Si ves: ',
              },
              {
                em: '"Access Tokens cannot be created because an organization requires SSO"',
              },
            ],
            [
              {
                text: 'Usa en su lugar un token por organización. Ejecuta en tu terminal:',
              },
            ],
            [
              {
                code: 'flyctl tokens org <your-org-name>',
              },
            ],
            [
              {
                text: 'Luego pega arriba el token generado.',
              },
            ],
          ],
          q: 'No se puede crear un Personal Access Token (error de SSO)',
        },
      },
      upgradeModel: 'Mejorar modelo',
      website: 'Sitio web',
    },
    changelog: {
      backHome: 'Volver al inicio',
      changeTypes: {
        breaking: 'Cambios importantes',
        feature: 'Añadido',
        fix: 'Corregido',
        other: 'Otros',
        performance: 'Rendimiento',
      },
      desc: 'Sigue cada actualización de LibreFang',
      downloads: 'descargas',
      empty: 'No se encontraron versiones.',
      filters: {
        all: 'Todas',
        prerelease: 'Preliminares',
        stable: 'Estables',
      },
      fullNotes: 'Consulta las notas completas de la versión en GitHub.',
      loadError: 'No se pudieron cargar las versiones',
      loading: 'Cargando versiones...',
      noFilterMatches: 'Ninguna versión coincide con el filtro actual.',
      releaseTypes: {
        beta: 'Beta',
        rc: 'RC',
        stable: 'Estable',
      },
      releases: 'versiones',
      title: 'Cambios',
      unknownError: 'Error desconocido',
      viewOnGitHub: 'Ver en GitHub',
    },
    footer: { docs: 'Documentación', license: 'Licencia', privacy: 'Privacidad', changelog: 'Cambios' },
  },
}

rawTranslations.uk = {
  nav: { architecture: 'Архітектура', hands: 'Hands', performance: 'Продуктивність', install: 'Встановлення', downloads: 'Завантаження', docs: 'Документація', features: 'Маркетплейс', evolution: 'Саморозвиток навичок', workflows: 'Робочі процеси', registry: 'Реєстр', learnMore: 'Можливості' },
  hero: {
    badge: 'Open Source',
    title1: 'Агентна',
    title2: 'операційна система',
    typing: [
      'запускайте автономних агентів 24/7',
      'замінюйте цілі робочі процеси',
      'розгортайте на будь-якому обладнанні',
      'контролюйте через 16 рівнів безпеки',
    ],
    desc: 'LibreFang — production-grade runtime для автономних AI-агентів. Один binary, {handsCount} вбудованих одиниць можливостей, {channelsCount} адаптерів каналів. Написано на Rust для навантажень, які не можуть дозволити собі простій.',
    getStarted: 'Почати',
    viewGithub: 'Переглянути на GitHub',
  },
  stats: { coldStart: 'Холодний старт', memory: 'Памʼять', security: 'Рівнів безпеки', channels: 'Канали', hands: 'Hands', providers: 'Провайдери' },
  architecture: {
    label: 'Дизайн системи',
    title: 'Пʼятирівнева архітектура',
    desc: 'Від обладнання до каналів для користувачів. Кожен рівень ізольований, тестований і замінний.',
    layers: [
      { label: 'Канали', desc: '44 адаптери: Telegram, Slack, Discord, Feishu, DingTalk, WhatsApp...' },
      { label: 'Hands', desc: '15 автономних одиниць можливостей із власними моделями та інструментами' },
      { label: 'Ядро', desc: 'Життєвий цикл агентів, оркестрація робочих процесів, контроль бюджету, планування' },
      { label: 'Runtime', desc: 'Асинхронний Tokio, WASM sandbox, Merkle audit chain, захист від SSRF' },
      { label: 'Обладнання', desc: 'Один binary: ноутбук, VPS, Raspberry Pi, bare metal, хмара' },
    ],
    kernelDescs: [
      'Створення, запуск, пауза, відновлення, зупинка, видалення',
      '9 вбудованих шаблонів, DAG-оркестрація',
      'Ліміти витрат на агента, глобальні обмеження, сповіщення',
      'Cron-тригери, інтервальні задачі, event hooks',
      'Короткострокова, довгострокова, епізодична, семантична памʼять, автоматична консолідація',
      'Python і prompt-навички, hot-reload',
      'Model Context Protocol, Agent-to-Agent',
      'Open Fang Protocol для mesh-мереж',
    ],
    runtimeDescs: [
      'Багатопотоковий асинхронний runtime',
      'Ізольоване виконання недовіреного коду',
      'Перевірка цілісності через hash-chain',
      'Блокування доступу до внутрішньої мережі',
      'Аналіз потоків даних на секрети',
      'Token bucket для кожного агента/каналу',
      'Багаторівнева санітизація введення',
      'Рольовий контроль доступу + audit log',
    ],
    hardwareDescs: [
      '32MB, без залежностей, просто скопіюйте й запустіть',
      'Нативні збірки для x86_64 та ARM64',
      'ARM64 — працює на Pi 4 із 64MB RAM',
      'Середовище Termux, нативний ARM64',
      'Будь-який VPS за $5/міс, Docker опційний',
      'Пряме розгортання, оркестратор не потрібен',
      'Нативний desktop app із системним треєм',
    ],
  },
  hands: {
    label: 'Одиниці можливостей',
    title: '15 вбудованих Hands',
    desc: 'Кожен Hand має власну модель, інструменти й робочий процес. Активуйте, а не збирайте вручну.',
    items: [
      { name: 'Clip', desc: 'YouTube у вертикальні shorts з AI-субтитрами. Автопублікація в Telegram.' },
      { name: 'Lead', desc: 'Щоденний пошук лідів з ICP-оцінкою, дедуплікацією та CSV-експортом.' },
      { name: 'Collector', desc: 'OSINT-рівень моніторингу розвідданих із виявленням змін.' },
      { name: 'Predictor', desc: 'Каліброване ймовірнісне прогнозування для ринків і результатів.' },
      { name: 'Researcher', desc: 'Глибоке дослідження з оцінкою надійності джерел.' },
      { name: 'Trader', desc: 'Автономний рушій ринкової аналітики й торгівлі — багатосигнальний аналіз, adversarial reasoning, керування ризиками.' },
    ],
    more: '+ ще 9: Twitter, Browser, Analytics, DevOps, Creator, LinkedIn, Reddit, Strategist, API Tester',
  },
  performance: {
    label: 'Бенчмарки',
    title: 'Збудовано інакше',
    desc: 'Rust, не TypeScript. Production, не прототип.',
    metric: 'Метрика',
    others: 'Інші',
    rows: [
      { metric: 'Холодний старт', others: '2.5 ~ 4s', librefang: '180ms' },
      { metric: 'Памʼять у простої', others: '180 ~ 250MB', librefang: '40MB' },
      { metric: 'Розмір binary', others: '100 ~ 200MB', librefang: '32MB' },
      { metric: 'Рівні безпеки', others: '2 ~ 3', librefang: '16' },
      { metric: 'Адаптери каналів', others: '8 ~ 15', librefang: '44' },
      { metric: 'Вбудовані Hands', others: '0', librefang: '15' },
    ],
  },
  install: {
    label: 'Почати',
    title: 'Одна команда',
    desc: 'Один binary. Без Docker. 60 секунд до автономних агентів.',
    terminal: 'термінал',
    comment: '# агенти вже працюють автономно',
    requires: 'Потрібно',
    includes: 'Містить',
    reqItems: ['Linux / macOS / Windows', 'Мінімум 64MB RAM', 'x86_64 або ARM64', 'LLM API-ключ'],
    incItems: ['{handsCount} вбудованих Hands', '{channelsCount} адаптерів каналів', '{providersCount} LLM-провайдерів', 'Desktop app (Tauri 2.0)'],
  },
  faq: {
    label: 'FAQ',
    title: 'Поширені питання',
    items: [
      { q: 'Що таке LibreFang?', a: 'Production-grade Agent Operating System, написана на Rust. Вона запускає автономних AI-агентів за розкладом 24/7 без обовʼязкових user prompts. Runtime, безпека й інфраструктура каналів в одному binary.' },
      { q: 'Що таке Hands?', a: 'Самодостатні автономні одиниці можливостей. Кожен Hand має власну модель, інструменти й робочий процес. 15 вбудованих: Clip (відео), Lead (пошук клієнтів), Collector (OSINT), Predictor (прогнозування), Researcher, Trader та інші.' },
      { q: 'Які LLM-провайдери підтримуються?', a: '50 провайдерів: Anthropic, OpenAI, Gemini, Groq, DeepSeek, Mistral, Together, Ollama, vLLM та інші. Загалом 200+ моделей. Кожен Hand може використовувати окремого провайдера.' },
      { q: 'Які канали підтримуються?', a: '44 адаптери каналів: Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Teams, Google Chat, Feishu, DingTalk, Mastodon, Bluesky, LinkedIn, Reddit, IRC та інші.' },
      { q: 'Чи готово це до production?', a: '2100+ тестів, нуль Clippy warnings. 16 рівнів безпеки, включно з WASM sandbox, Merkle audit chain і SSRF protection. До v1.0 рекомендовано фіксувати версію для стабільності.' },
    ],
  },
  community: {
    label: 'Open Source',
    title: 'Приєднуйтесь до спільноти',
    desc: 'LibreFang розробляється відкрито. Надсилайте код, повідомляйте про проблеми або долучайтесь до обговорення.',
    items: [
      { label: 'Долучитися', desc: 'Надсилайте PR, виправляйте баги, покращуйте документацію' },
      { label: 'Повідомити', desc: 'Знайшли баг? Відкрийте issue' },
      { label: 'Обговорити', desc: 'Ставте питання, діліться ідеями' },
      { label: 'Discord', desc: 'Приєднуйтесь до нашого Discord-сервера' },
    ],
    open: 'Відкрити',
  },
  meta: {
    title: 'LibreFang - агентна операційна система',
    description: 'LibreFang — production-grade Agent Operating System, написана на Rust. 180ms холодний старт, 40MB памʼяті, 16 рівнів безпеки, 44 адаптери каналів. Запускайте автономних AI-агентів 24/7.',
  },
  workflows: {
    label: 'Робочі процеси',
    title: 'Замінюйте цілі робочі процеси',
    desc: 'LibreFang не просто допомагає — він бере роботу на себе. Це операції, для яких інакше довелось би наймати людей.',
    items: [
      { title: 'Контентний pipeline', desc: 'Clip + Twitter: моніторинг трендових відео, нарізання shorts, додавання субтитрів, публікація в соцмережі — усе поки ви offline.' },
      { title: 'Пошук продажних лідів', desc: 'Lead запускається щоночі: знаходить потенційних клієнтів, оцінює ICP-fit, прибирає дублікати, експортує чистий CSV.' },
      { title: 'Конкурентна розвідка', desc: 'Collector стежить за сайтами конкурентів, цінами, вакансіями й новинами. Сповіщає в момент, коли щось змінюється.' },
      { title: 'Мультиагентна оркестрація', desc: 'Обʼєднуйте Hands у workflow: Researcher → Predictor → Clip → трансляція в 44 канали.' },
      { title: 'Міграція', desc: 'Одна команда: librefang migrate --from openclaw. Агенти, памʼять і навички переносяться автоматично.' },
      { title: 'Production-безпека', desc: 'WASM sandbox, Merkle audit chain, SSRF protection, prompt injection scanning, GCRA rate limiting — 16 рівнів.' },
    ],
  },
  docs: {
    label: 'Документація',
    title: 'Документація',
    desc: 'Комплексні гайди для LibreFang',
    categories: [
      { title: 'Огляд', desc: 'Вступ, швидкий старт, архітектура' },
      { title: 'Автоматизація', desc: 'Cron-задачі, webhooks, інтеграції' },
      { title: 'Інфраструктура', desc: 'Розгортання, моніторинг, масштабування' },
    ],
    viewAll: 'Переглянути всю документацію',
  },
  githubStats: {
    label: 'Спільнота',
    title: 'Приєднуйтесь до спільноти',
    desc: 'Допоможіть нам будувати майбутнє автономних AI-агентів',
    stars: 'Зірки', forks: 'Форки', issues: 'Issues', prs: 'PR',
    downloads: 'Завантаження', docsVisits: 'Візити в документацію', lastUpdate: 'Останнє оновлення',
    starHistory: 'Історія зірок', starUs: 'Поставити зірку', discuss: 'Обговорити',
  },
  contributing: {
    label: 'Участь',
    title: 'Як долучитися',
    desc: 'LibreFang — open source проєкт і вітає будь-які внески.',
    steps: [
      { title: 'Fork і clone', desc: 'Зробіть fork репозиторію й склонуйте його локально, щоб почати.' },
      { title: 'Оберіть issue', desc: 'Перегляньте відкриті issues з мітками "good first issue" або "help wanted".' },
      { title: 'Надішліть PR', desc: 'Внесіть зміни, напишіть тести й надішліть pull request на review.' },
    ],
    cta: 'Прочитати гайд для контрибʼюторів',
  },
  evolution: {
    label: 'Саморозвиток навичок',
    title: 'Агенти, які навчаються самі',
    desc: 'Після складного завдання фоновий LLM-review вирішує, чи варто зберегти підхід. Нові навички hot-reload-яться в runtime без перезапуску.',
    tagline: 'Автономно. З версіями. Із security scan.',
    toolsHeading: 'Інструменти еволюції',
    howItWorks: [
      { title: 'Автоматичне виявлення', desc: '5+ викликів інструментів запускають фоновий review підходу.' },
      { title: 'Hot-reload', desc: 'Нові й оновлені навички доступні одразу — без перезапуску daemon.' },
      { title: 'Сканування безпеки', desc: 'Кожна зміна проходить prompt injection detection з автоматичним rollback.' },
      { title: 'Історія версій', desc: 'До 10 версій на навичку з timestamps, changelog і rollback snapshots.' },
    ],
    tools: [
      { name: 'skill_evolve_create', desc: 'Збережіть успішний підхід як нову prompt-only навичку.' },
      { name: 'skill_evolve_update', desc: 'Повністю перепишіть prompt context навички.' },
      { name: 'skill_evolve_patch', desc: 'Точковий find-and-replace із 5-стратегійним fuzzy matching.' },
      { name: 'skill_evolve_rollback', desc: 'Миттєво поверніться до попередньої версії.' },
      { name: 'skill_evolve_write_file', desc: 'Додайте допоміжні файли: references, templates, scripts, assets.' },
      { name: 'skill_evolve_delete', desc: 'Видаліть локально створену навичку.' },
    ],
    cta: 'Прочитати документацію з еволюції навичок',
  },
  registry: {
    label: 'Реєстр',
    total: 'усього',
    matching: 'збігів',
    all: 'Усі',
    searchPlaceholder: 'Шукайте за назвою, id або тегом...',
    loading: 'Завантаження реєстру…',
    errorTitle: 'Не вдалося завантажити реєстр',
    errorDesc: 'Спрацював GitHub rate limit або proxy недоступний. Спробуйте ще раз за кілька секунд.',
    emptyTitle: 'Тут поки нічого немає',
    emptyDesc: 'Цей розділ реєстру ще не заповнений. Перевірте пізніше або додайте власний запис.',
    contribute: 'Долучитися на GitHub',
    noMatches: 'Немає збігів для',
    backHome: 'Головна',
    sourceHint: 'Дані проксійовані з репозиторію librefang-registry на GitHub.',
    readDocs: 'Читати документацію',
    manifest: 'Маніфест',
    copy: 'Копіювати',
    manifestErrorTitle: 'Не вдалося завантажити маніфест',
    allIn: 'Усі {category}',
    useIt: 'Використати',
    configOnly: 'Записи {category} налаштовуються через ~/.librefang/config.toml, а не через CLI install command. Скопіюйте маніфест нижче й вставте його у відповідний розділ config.',
    relatedIn: 'Більше {category}',
    retry: 'Повторити',
    openInDashboard: 'Або встановіть через локальний dashboard',
    lastUpdated: 'Оновлено',
    copyLink: 'Копіювати посилання на цей розділ',
    trending: 'У тренді',
    downloads: 'завантажень',
    stars: 'зірок',
    thisWeek: 'цього тижня',
    sort: { label: 'Сортувати', popular: 'Популярні', nameAsc: 'Назва A–Z', nameDesc: 'Назва Z–A', trending: 'Найбільше кліків', downloads: 'Найбільше завантажень', weekly: 'У тренді цього тижня' },
    onThisPage: 'На цій сторінці',
    previous: 'Попередній',
    next: 'Наступний',
    prevNext: 'Попередній / наступний у категорії',
    readme: 'README',
    viewHistory: 'Історія',
    templateDiff: 'Diff шаблону',
    subcategories: {
      ai: 'AI', business: 'Бізнес', cloud: 'Хмара', communication: 'Комунікація',
      content: 'Контент', creation: 'Створення', data: 'Дані', developer: 'Розробник',
      development: 'Розробка', devtools: 'DevTools', email: 'Email',
      engineering: 'Інженерія', enterprise: 'Enterprise', iot: 'IoT',
      language: 'Мова', messaging: 'Повідомлення', productivity: 'Продуктивність',
      research: 'Дослідження', skills: 'Навички', social: 'Соцмережі', thinking: 'Мислення',
    },
    categories: {
      skills: { title: 'Навички', desc: 'Підключувані набори інструментів — Python, WASM, Node або prompt-only skills, які розширюють можливості агента.' },
      mcp:    { title: 'MCP-сервери', desc: 'Model Context Protocol servers, які підключають зовнішні інструменти й джерела даних безпосередньо до будь-якого агента.' },
      plugins:{ title: 'Плагіни', desc: 'Runtime extensions, які додають власні команди, канали або поведінку до LibreFang daemon.' },
      hands:  { title: 'Hands', desc: 'Автономні одиниці можливостей. Кожен Hand має власну модель, інструменти й workflow — активуйте, не збирайте.' },
      agents: { title: 'Агенти', desc: 'Готові шаблони агентів. Модель, system prompt, capabilities і schedule — усе в одному маніфесті.' },
      providers: { title: 'Провайдери', desc: 'Адаптери LLM-провайдерів. Anthropic, OpenAI, Gemini, Groq, локальні моделі — і ще 40+ між ними.' },
      workflows: { title: 'Робочі процеси', desc: 'Багатокрокова оркестрація агентів у TOML. Зʼєднуйте агентів, розгалужуйте за умовами, зберігайте стан.' },
      channels:  { title: 'Канали', desc: 'Адаптери повідомлень. Telegram, Slack, Discord, WhatsApp, LINE і 40+ інших платформ.' },
    },
  },
  search: {
    title: 'Пошук у реєстрі',
    placeholder: 'Шукайте навички, Hands, агентів, провайдерів…',
    close: 'Закрити',
    noResults: 'Немає збігів для "{query}"',
    hint: 'Почніть вводити, щоб шукати серед усіх записів реєстру.',
    kbd: '↑↓ навігація · ↵ відкрити · esc закрити',
    open: 'Пошук',
  },
  browse: {
    title: 'Огляд реєстру',
    desc: 'Усі категорії одним поглядом — оберіть категорію, щоб побачити всі записи, відсортовані за популярністю.',
  },
  notFound: {
    title: 'Сторінку не знайдено',
    desc: 'Ми не знайшли те, що ви шукали.',
    home: 'Повернутися на головну',
  },
  pwa: {
    title: 'Встановити LibreFang',
    desc: 'Додайте сайт на головний екран або в dock.',
    install: 'Встановити',
    dismiss: 'Закрити',
  },
  common: {
    copy: 'Копіювати',
    copied: 'Скопійовано!',
    copyToClipboard: 'Копіювати в буфер',
    loadingReleases: 'Завантаження релізів...',
    skipToContent: 'Перейти до вмісту',
    backToTop: 'Нагору',
    viewFull: 'Переглянути повністю',
    contributors: 'Учасники',
    agentOs: 'ОС для агентів',
    breadcrumb: 'Навігаційний ланцюжок',
  },
  header: {
    toggleTheme: 'Перемкнути тему',
    viewSource: 'Переглянути джерело на GitHub',
    source: 'Джерело',
    switchLanguage: 'Змінити мову',
    switchLanguageNamed: 'Змінити мову ({name})',
    toggleMenu: 'Перемкнути меню',
  },
  errorBoundary: {
    label: 'Помилка виконання',
    title: 'Щось пішло не так.',
    reload: 'Перезавантажити',
    home: 'На головну',
  },
  metrics: {
    breadcrumb: 'Метрики',
    label: 'Метрики реєстру',
    title: 'Телеметрія кліків',
    desc: 'Агреговані лічильники переглядів сторінок деталей у реєстрі, записані через endpoint worker /api/registry/click.',
    descPrefix: 'Агреговані лічильники переглядів сторінок деталей у реєстрі, записані через endpoint worker ',
    endpoint: '/api/registry/click',
    descSuffix: '.',
    loadError: 'Не вдалося завантажити метрики',
    totalClicks: 'Усього кліків',
    uniqueItems: 'Унікальні елементи',
    activeCategories: 'Активні категорії',
    generated: 'Згенеровано',
    perCategory: 'За категоріями',
    topItems: 'Найпопулярніші елементи',
    items: 'елементів',
    clicks: 'кліків',
  },
  deploy: {
    title: 'Розгорнути LibreFang',
    subtitle: 'Оберіть платформу',
    terminalIntro: 'Або розгорніть із термінала:',
    installLocally: 'Встановити локально',
    website: 'Сайт',
    copyrightSuffix: 'Операційна система для агентів',
    backToPlatforms: 'Назад до платформ',
    badges: ['Безкоштовна LLM включена', 'API-ключ не потрібен', '1 ГБ сховища'],
    tokenRequired: 'Введіть API-токен Fly.io.',
    deployFailed: 'Розгортання не вдалося',
    deploying: 'Розгортання...',
    deployToFly: 'Розгорнути на Fly.io',
    deployed: 'Розгорнуто!',
    deployedDesc: 'Ваш екземпляр LibreFang запускається (1-2 хв). Безкоштовна LLM (Step 3.5 Flash) уже налаштована й готова до використання.',
    openDashboard: 'Відкрити панель',
    flyConsole: 'Консоль Fly.io',
    app: 'Застосунок',
    region: 'Регіон',
    model: 'Модель',
    includedModel: 'Step 3.5 Flash (безкоштовно)',
    upgradeModel: 'Оновити модель',
    freeNote: 'Безкоштовна LLM (Step 3.5 Flash через OpenRouter) уже налаштована. Ваш екземпляр працює одразу — API-ключі не потрібні.',
    stepOneTitle: 'Отримайте API-токен Fly.io',
    stepOnePrefix: '',
    signUp: 'Зареєструйтесь',
    or: 'або',
    logIn: 'увійдіть',
    stepOneMiddle: 'у Fly.io, потім перейдіть до',
    personalAccessTokens: 'Personal Access Tokens',
    stepOneSuffix: 'і створіть новий токен.',
    stepTwoTitle: 'Вставте токен і розгорніть',
    stepTwoDesc: 'Ваш токен надсилається лише до API Fly.io і ніколи не зберігається на наших серверах.',
    tokenLabel: 'API-токен Fly.io',
    troubleshooting: 'Усунення проблем',
    platforms: {
      flyio: { desc: 'Безкоштовно назавжди, постійне сховище', badge: 'Рекомендовано', demo: 'Жива демоверсія' },
      render: { desc: 'Розгортання OAuth в один клік', badge: 'Найпростіше', demo: 'Жива демоверсія', warning: 'Безкоштовний тариф: засинає після 15 хв, без постійного сховища' },
      railway: { desc: 'Просте розгортання з $5 безкоштовного кредиту' },
      gcp: { desc: 'Безкоштовно назавжди (e2-micro), 30 ГБ сховища', badge: 'Terraform' },
      docker: { desc: 'Одна команда, працює будь-де' },
    },
    localInstalls: {
      macos: { name: 'macOS', desc: 'Homebrew або завантаження binary' },
      linux: { name: 'Linux', desc: 'Скрипт встановлення або завантаження binary' },
      windows: { name: 'Windows', desc: 'Інсталятор PowerShell або .msi' },
    },
    steps: {
      auth: 'Перевірка токена...',
      app: 'Створення застосунку...',
      net: 'Виділення IP-адрес...',
      vol: 'Створення постійного тому...',
      machine: 'Запуск машини зі Step 3.5 Flash...',
    },
    troubleshootingItems: {
      sso: {
        q: 'Не вдається створити Personal Access Token (помилка SSO)',
        lines: [
          [
            { text: 'Якщо бачите: ' },
            { em: '"Access Tokens cannot be created because an organization requires SSO"' },
          ],
          [{ text: 'Використайте токен організації. Запустіть у терміналі:' }],
          [{ code: 'flyctl tokens org <your-org-name>' }],
          [{ text: 'Потім вставте згенерований токен вище.' }],
        ],
      },
      image: {
        q: 'Розгортання не вдалося: image not found',
        lines: [
          [
            { text: 'Docker image ' },
            { code: 'ghcr.io/librefang/librefang:latest' },
            { text: ' збирається для кожного релізу.' },
          ],
          [{ text: 'Якщо реліз ще не опубліковано, використайте terminal deploy script нижче — він збирає з джерел.' }],
        ],
      },
      llm: {
        q: 'Як додати або змінити LLM-провайдера після розгортання?',
        lines: [
          [{ code: 'flyctl secrets set OPENAI_API_KEY=sk-... --app your-app-name' }],
          [
            { text: 'Потім відредагуйте ' },
            { code: '/data/config.toml' },
            { text: ' через ' },
            { code: 'flyctl ssh console' },
            { text: ', щоб змінити модель за замовчуванням.' },
          ],
        ],
      },
    },
  },
  changelog: {
    title: 'Журнал змін',
    desc: 'Відстежуйте кожне оновлення LibreFang',
    backHome: 'Назад на головну',
    releases: 'релізів',
    downloads: 'завантажень',
    loading: 'Завантаження релізів...',
    loadError: 'Не вдалося завантажити релізи',
    unknownError: 'Невідома помилка',
    empty: 'Релізів не знайдено.',
    noFilterMatches: 'Жоден реліз не відповідає поточному фільтру.',
    fullNotes: 'Повні нотатки до релізу дивіться на GitHub.',
    viewOnGitHub: 'Переглянути на GitHub',
    filters: { all: 'Усі', stable: 'Стабільні', prerelease: 'Передрелізи' },
    releaseTypes: { stable: 'Стабільний', rc: 'RC', beta: 'Бета' },
    changeTypes: {
      feature: 'Додано',
      fix: 'Виправлено',
      breaking: 'Несумісні зміни',
      performance: 'Продуктивність',
      other: 'Інше',
    },
  },
  footer: { docs: 'Документація', license: 'Ліцензія', privacy: 'Приватність', changelog: 'Журнал змін' },
}

function mergeTranslation(base: Translation, override: Translation): Translation {
  return mergeObject(base, override) as Translation
}

function mergeObject(base: unknown, override: unknown): unknown {
  if (Array.isArray(base) || Array.isArray(override)) {
    return override ?? base
  }
  if (
    base &&
    override &&
    typeof base === 'object' &&
    typeof override === 'object'
  ) {
    const merged: Record<string, unknown> = { ...(base as Record<string, unknown>) }
    for (const [key, value] of Object.entries(override as Record<string, unknown>)) {
      merged[key] = mergeObject(merged[key], value)
    }
    return merged
  }
  return override ?? base
}

const translationCache = new Map<string, Translation>()

export function getTranslation(lang: string): Translation {
  const english = rawTranslations.en!
  const selected = rawTranslations[lang]
  if (!selected || selected === english) return english
  const cached = translationCache.get(lang)
  if (cached) return cached
  const merged = mergeTranslation(english, selected)
  translationCache.set(lang, merged)
  return merged
}
