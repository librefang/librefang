import { render, screen } from "@testing-library/react";
import type { UseQueryResult } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import type { AgentKvPair } from "../../../api";
import { KV_TITLE_TRUNCATE, KV_VALUE_TRUNCATE } from "../constants";
import { AgentKvRows } from "./AgentKvRows";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

function renderRows(kvQuery: Partial<UseQueryResult<AgentKvPair[]>>) {
  return render(
    <table>
      <tbody>
        <AgentKvRows kvQuery={kvQuery as UseQueryResult<AgentKvPair[]>} />
      </tbody>
    </table>,
  );
}

describe("AgentKvRows", () => {
  it("shows generic copy instead of exposing a raw query error", () => {
    renderRows({ isError: true, error: new Error("internal path: /srv/private.db") });

    expect(screen.getByText("common.error")).toBeInTheDocument();
    expect(screen.queryByText(/private\.db/)).not.toBeInTheDocument();
  });

  it("caps both cell and hover previews with the same ellipsis semantics", () => {
    const value = "x".repeat(KV_TITLE_TRUNCATE + 100);
    renderRows({
      isError: false,
      isLoading: false,
      data: [
        {
          key: "long-value",
          value,
          source: "test",
        },
      ],
    });

    const valueCell = screen.getByTitle(`${"x".repeat(KV_TITLE_TRUNCATE)}…`);
    expect(valueCell).toHaveTextContent(`${"x".repeat(KV_VALUE_TRUNCATE)}…`);
    expect(valueCell).not.toHaveAttribute("title", value);
  });
});
