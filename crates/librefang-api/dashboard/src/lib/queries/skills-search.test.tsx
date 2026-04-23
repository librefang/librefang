import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import type { ClawHubBrowseResponse, ClawHubSkillDetail } from "../../api";
import {
  useClawHubSearch,
  useClawHubSkill,
  useSkillHubSearch,
  useSkillHubSkill,
} from "./skills";
import * as httpClient from "../http/client";
import { clawhubKeys, skillhubKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  clawhubSearch: vi.fn(),
  clawhubGetSkill: vi.fn(),
  skillhubSearch: vi.fn(),
  skillhubGetSkill: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useClawHubSearch", () => {
  it("should be disabled when query is empty string", () => {
    const { result } = renderHook(() => useClawHubSearch(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResults);
    expect(httpClient.clawhubSearch).toHaveBeenCalledWith("test");
  });

  it("should use the correct queryKey", async () => {
    const mockResults: ClawHubBrowseResponse = { items: [] };
    const { queryClient, wrapper } = createQueryClientWrapper();
    vi.mocked(httpClient.clawhubSearch).mockResolvedValue(mockResults);

    const { result } = renderHook(() => useClawHubSearch("test"), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(queryClient.getQueryData(clawhubKeys.search("test"))).toEqual(mockResults);
  });
});

describe("useClawHubSkill", () => {
  it("should be disabled when slug is empty string", () => {
    const { result } = renderHook(() => useClawHubSkill(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockSkill);
    expect(httpClient.clawhubGetSkill).toHaveBeenCalledWith("my-skill");
  });

  it("should use the correct queryKey", async () => {
    const mockSkill: ClawHubSkillDetail = { slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" };
    const { queryClient, wrapper } = createQueryClientWrapper();
    vi.mocked(httpClient.clawhubGetSkill).mockResolvedValue(mockSkill);

    const { result } = renderHook(() => useClawHubSkill("my-skill"), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(queryClient.getQueryData(clawhubKeys.detail("my-skill"))).toEqual(mockSkill);
  });
});

describe("useSkillHubSearch", () => {
  it("should be disabled when query is empty string", () => {
    const { result } = renderHook(() => useSkillHubSearch(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockResults);
    expect(httpClient.skillhubSearch).toHaveBeenCalledWith("test");
  });

  it("should use the correct queryKey", async () => {
    const mockResults: ClawHubBrowseResponse = { items: [] };
    const { queryClient, wrapper } = createQueryClientWrapper();
    vi.mocked(httpClient.skillhubSearch).mockResolvedValue(mockResults);

    const { result } = renderHook(() => useSkillHubSearch("test"), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(queryClient.getQueryData(skillhubKeys.search("test"))).toEqual(mockResults);
  });
});

describe("useSkillHubSkill", () => {
  it("should be disabled when slug is empty string", () => {
    const { result } = renderHook(() => useSkillHubSkill(""), {
      wrapper: createQueryClientWrapper().wrapper,
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
      wrapper: createQueryClientWrapper().wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data).toEqual(mockSkill);
    expect(httpClient.skillhubGetSkill).toHaveBeenCalledWith("my-skill");
  });

  it("should use the correct queryKey", async () => {
    const mockSkill: ClawHubSkillDetail = { slug: "my-skill", name: "My Skill", description: "desc", version: "1.0.0", author: "tester", stars: 0, downloads: 0, tags: [], readme: "# My Skill" };
    const { queryClient, wrapper } = createQueryClientWrapper();
    vi.mocked(httpClient.skillhubGetSkill).mockResolvedValue(mockSkill);

    const { result } = renderHook(() => useSkillHubSkill("my-skill"), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(queryClient.getQueryData(skillhubKeys.detail("my-skill"))).toEqual(mockSkill);
  });
});
