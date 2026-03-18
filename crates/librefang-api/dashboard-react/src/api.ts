export interface HealthResponse {
  status?: string;
}

export interface StatusResponse {
  version?: string;
  agent_count?: number;
  uptime_seconds?: number;
}

export interface ProviderItem {
  id: string;
  display_name?: string;
  auth_status?: string;
  reachable?: boolean;
  model_count?: number;
  latency_ms?: number;
}

export interface ChannelItem {
  name: string;
  display_name?: string;
  configured?: boolean;
  has_token?: boolean;
}

export interface SkillsResponse {
  skills?: Array<{ name: string }>;
}

export interface DashboardSnapshot {
  health: HealthResponse;
  status: StatusResponse;
  providers: ProviderItem[];
  channels: ChannelItem[];
  skillCount: number;
}

type Json = Record<string, unknown>;

function authHeader(): HeadersInit {
  const token = localStorage.getItem("librefang-api-key") || "";
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: {
      "Content-Type": "application/json",
      ...authHeader()
    }
  });
  if (!response.ok) {
    const text = await response.text();
    let message = response.statusText;
    try {
      const json = JSON.parse(text) as Json;
      if (typeof json.error === "string") {
        message = json.error;
      }
    } catch {
      // ignore parse errors
    }
    throw new Error(message || `HTTP ${response.status}`);
  }
  return (await response.json()) as T;
}

export async function loadDashboardSnapshot(): Promise<DashboardSnapshot> {
  const [health, status, providersRaw, channelsRaw, skillsRaw] = await Promise.all([
    get<HealthResponse>("/api/health"),
    get<StatusResponse>("/api/status"),
    get<{ providers?: ProviderItem[] }>("/api/providers"),
    get<{ channels?: ChannelItem[] }>("/api/channels"),
    get<SkillsResponse>("/api/skills")
  ]);

  return {
    health,
    status,
    providers: providersRaw.providers ?? [],
    channels: channelsRaw.channels ?? [],
    skillCount: skillsRaw.skills?.length ?? 0
  };
}
