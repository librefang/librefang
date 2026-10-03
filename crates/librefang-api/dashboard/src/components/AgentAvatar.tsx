import { type HTMLAttributes, memo } from "react";
import { Avatar, type AvatarSize } from "./ui/Avatar";
import { useAgentAvatarUrl } from "../lib/queries/agents";

interface AgentAvatarProps extends HTMLAttributes<HTMLDivElement> {
  agentId: string;
  /**
   * The agent's `identity.avatar_url` — `/api/agents/{id}/avatar` when it has
   * an image, absent otherwise.
   *
   * Two primitives rather than the whole `identity` block on purpose. One of
   * the callers sits inside a `memo`ised message bubble whose parent re-renders
   * with a freshly-built agent object on every poll (the payload carries
   * `last_active`, which advances while the agent works), so an object prop
   * would hand that bubble a new reference every 30 s and defeat the memo.
   */
  avatarUrl?: string;
  /** The agent's `identity.emoji`, shown when there is no image. */
  emoji?: string;
  /** The agent's name — what `Avatar` turns into initials when there is no image. */
  fallback: string;
  size?: AvatarSize;
}

/**
 * An agent's identity — image, then emoji, then initials — anywhere the agent
 * is shown (#8339).
 *
 * This exists so the avatar can be rendered from a plain `.map()`: the image
 * has to be fetched with the bearer credential and handed to `<img>` as an
 * object URL, which is a hook (`useAgentAvatarUrl`), and `AgentsPage`'s
 * `renderAgentRow` is an ordinary function invoked inside `map()` — calling
 * the hook there would break the Rules of Hooks. Wrapping it in a component
 * moves the hook into a real render.
 *
 * `identity.avatar_url` is always `/api/agents/{id}/avatar` when set (it is
 * derived from the id, never read back), so its presence is exactly the "is
 * there an image to fetch" gate `useAgentAvatarUrl` wants. Passing it as `src`
 * to a bare `<img>` would 401: that route is authenticated, and an `<img>` tag
 * cannot attach the bearer header.
 */
export const AgentAvatar = memo(function AgentAvatar({
  agentId,
  avatarUrl,
  emoji,
  fallback,
  size = "md",
  ...props
}: AgentAvatarProps) {
  const src = useAgentAvatarUrl(agentId, !!avatarUrl);
  return <Avatar fallback={fallback} size={size} src={src} emoji={emoji} {...props} />;
});
