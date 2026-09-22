import { useEffect, useState } from "react";

/**
 * An object URL for `blob`, revoked when the blob changes or the caller
 * unmounts.
 *
 * The blob and the URL have different lifetimes and cannot be kept in one
 * place. The blob is owned by the query cache and is shared — every row showing
 * the same agent gets the same bytes. The URL is a document-scoped handle that
 * leaks until it is revoked, so it belongs to the component that is currently
 * painting it and is minted in an effect keyed on the blob.
 *
 * The cleanup resets to `undefined` as well as revoking. Without that, the next
 * paint still points an `<img>` at a URL that has just been revoked, which
 * renders as a broken image rather than as the initials the fallback is there
 * to give.
 *
 * Returns `undefined` while there is no blob, which is what `Avatar`'s `src`
 * wants: it falls back to the emoji and then to the initials on its own, so
 * there is no separate loading state to thread through the UI.
 */
export function useObjectUrl(blob: Blob | undefined): string | undefined {
  const [objectUrl, setObjectUrl] = useState<string | undefined>(undefined);

  useEffect(() => {
    if (!blob) {
      setObjectUrl(undefined);
      return;
    }
    const url = URL.createObjectURL(blob);
    setObjectUrl(url);
    return () => {
      URL.revokeObjectURL(url);
      setObjectUrl(undefined);
    };
  }, [blob]);

  return objectUrl;
}
