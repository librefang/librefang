import { queryOptions, useQuery } from "@tanstack/react-query";
import { listChatCommands, type ChatCommand } from "../http/client";
import { chatCommandKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

// The catalog is a static registry on the server plus the installed skill
// list, so it only moves when a skill is installed or removed. Long stale
// time, no polling.
const STALE_MS = 5 * 60_000;

export const chatCommandQueries = {
  list: () =>
    queryOptions({
      queryKey: chatCommandKeys.list(),
      queryFn: () => listChatCommands(),
      staleTime: STALE_MS,
    }),
};

/** Slash commands the chat can offer, served by `GET /api/commands`. */
export function useChatCommands(options: QueryOverrides = {}) {
  return useQuery(withOverrides(chatCommandQueries.list(), options));
}

export type { ChatCommand };
