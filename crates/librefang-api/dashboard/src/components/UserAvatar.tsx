import { type HTMLAttributes, memo } from "react";
import { Avatar, type AvatarSize } from "./ui/Avatar";
import { useUserAvatarUrl } from "../lib/queries/users";

interface UserAvatarProps extends HTMLAttributes<HTMLDivElement> {
  /**
   * The signed-in user's name.
   *
   * Doubles as the cache key and as the initials the fallback derives, and it
   * is deliberately not a path segment: the request goes to the literal
   * `/api/users/me/avatar` and the daemon resolves `me` from the credential.
   * See `currentUserAvatarPath` for why that matters — a name is
   * client-controlled, and a path built from one could only be admitted to the
   * authenticated-image allowlist by loosening it.
   */
  name: string;
  /** The user's emoji, shown when there is no image. */
  emoji?: string;
  /**
   * `whoami.has_avatar` — whether the daemon has a picture on disk for them.
   *
   * Optional, and `undefined` means "the daemon did not say" rather than "no":
   * only `false` skips the request. See `WhoamiResponse.has_avatar` for why the
   * two must not be conflated.
   */
  hasAvatar?: boolean;
  size?: AvatarSize;
}

/**
 * The signed-in user's identity — image, then emoji, then initials (#8339).
 *
 * The mirror of `AgentAvatar`, and it exists for the same reason: the image is
 * fetched with the bearer credential and handed to an `img` as an object URL,
 * which is a hook, so it needs a component to live in.
 *
 * `hasAvatar` gates the fetch exactly as `agentAvatarUrl` does on the agent
 * side, and it is free here: the chat fetches `whoami` anyway for the caller's
 * name and emoji.
 */
export const UserAvatar = memo(function UserAvatar({
  name,
  emoji,
  hasAvatar,
  size = "md",
  ...props
}: UserAvatarProps) {
  const src = useUserAvatarUrl(name, hasAvatar !== false);
  return <Avatar fallback={name} size={size} src={src} emoji={emoji} {...props} />;
});
