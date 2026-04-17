import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { ClawHubBrowseResponse, ClawHubSkillDetail } from "../../api";
import {
  useClawHubSearch,
  useClawHubSkill,
  useSkillHubSearch,
  useSkillHubSkill,
} from "./skills";
import * as httpClient from "../http/client";
import { clawhubKeys, skillhubKeys } from "./keys";

vi.mock("../http/client", () => ({
  clawhubSearch: vi.fn(),
  clawhubGetSkill: vi.fn(),
  skillhubSearch: vi.fn(),
  skillhubGetSkill: vi.fn(),
}));

function createQueryClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

function createWrapper(qc: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useClawHubSearch", () => {
  it("should be disabled when query is empty string", () => {
    const { result } = renderHook(() => useClawHubSearch(""), {
      wrapper: createWrapper(createQueryClient()),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.clawhubSearch).not.toHaveBeenCalled();
  });

  it("should fetch when query is valid", async () => {
    const mockResults: ClawHubBrowseResponse = { items: [{ slug: "skill-a", name: "Skill A", description: "desc", version: "1.0.0" }] };
    vi.mocked(httpClient.clawhubSearch).mockResolvedValue(mockResults);

    const { result } = renderHook(() => useClawHubSearch("test"), {
      wrapper: createWrapper(createQueryClient()),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResults);
    expect(httpClient.clawhubSearch).toHaveBeenCalledWith("test");
  });

  it("should use the correct queryKey", async () => {
    const qc = createQueryClient();
    vi.mocked(httpClient.clawhubSearch).mockResolvedValue({ items: [] });

    const { result } = renderHook(() => useClawHubSearch("test"), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = qc.getQueryCache().find({ queryKey: clawhubKeys.search("test") });
    expect(cached).toBeDefined();
  });
});

describe("useClawHubSkill", () => {
  it("should be disabled when slug is empty string", () => {
    const { result } = renderHook(() => useClawHubSkill(""), {
      wrapper: createWrapper(createQueryClient()),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.clawhubGetSkill).not.toHaveBeenCalled();
  });

  it("should fetch when slug is valid", async () => {
    const mockSkill: ClawHubSkillDetail = { slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" };
    vi.mocked(httpClient.clawhubGetSkill).mockResolvedValue(mockSkill);

    const { result } = renderHook(() => useClawHubSkill("my-skill"), {
      wrapper: createWrapper(createQueryClient()),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockSkill);
    expect(httpClient.clawhubGetSkill).toHaveBeenCalledWith("my-skill");
  });

  it("should use the correct queryKey", async () => {
    const qc = createQueryClient();
    vi.mocked(httpClient.clawhubGetSkill).mockResolvedValue({ slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" });

    const { result } = renderHook(() => useClawHubSkill("my-skill"), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = qc.getQueryCache().find({ queryKey: clawhubKeys.detail("my-skill") });
    expect(cached).toBeDefined();
  });
});

describe("useSkillHubSearch", () => {
  it("should be disabled when query is empty string", () => {
    const { result } = renderHook(() => useSkillHubSearch(""), {
      wrapper: createWrapper(createQueryClient()),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.skillhubSearch).not.toHaveBeenCalled();
  });

  it("should fetch when query is valid", async () => {
    const mockResults: ClawHubBrowseResponse = { items: [{ slug: "skill-b", name: "Skill B", description: "desc", version: "1.0.0" }] };
    vi.mocked(httpClient.skillhubSearch).mockResolvedValue(mockResults);

    const { result } = renderHook(() => useSkillHubSearch("test"), {
      wrapper: createWrapper(createQueryClient()),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResults);
    expect(httpClient.skillhubSearch).toHaveBeenCalledWith("test");
  });

  it("should use the correct queryKey", async () => {
    const qc = createQueryClient();
    vi.mocked(httpClient.skillhubSearch).mockResolvedValue({ items: [] });

    const { result } = renderHook(() => useSkillHubSearch("test"), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = qc.getQueryCache().find({ queryKey: skillhubKeys.search("test") });
    expect(cached).toBeDefined();
  });
});

describe("useSkillHubSkill", () => {
  it("should be disabled when slug is empty string", () => {
    const { result } = renderHook(() => useSkillHubSkill(""), {
      wrapper: createWrapper(createQueryClient()),
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(httpClient.skillhubGetSkill).not.toHaveBeenCalled();
  });

  it("should fetch when slug is valid", async () => {
    const mockSkill: ClawHubSkillDetail = { slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" };
    vi.mocked(httpClient.skillhubGetSkill).mockResolvedValue(mockSkill);

    const { result } = renderHook(() => useSkillHubSkill("my-skill"), {
      wrapper: createWrapper(createQueryClient()),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockSkill);
    expect(httpClient.skillhubGetSkill).toHaveBeenCalledWith("my-skill");
  });

  it("should use the correct queryKey", async () => {
    const qc = createQueryClient();
    vi.mocked(httpClient.skillhubGetSkill).mockResolvedValue({ slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" });

    const { result } = renderHook(() => useSkillHubSkill("my-skill"), {
      wrapper: createWrapper(qc),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = qc.getQueryCache().find({ queryKey: skillhubKeys.detail("my-skill") });
    expect(cached).toBeDefined();
  });
});
