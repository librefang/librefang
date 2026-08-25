import { useEffect, useState, type AnchorHTMLAttributes, type ImgHTMLAttributes } from "react";
import { fetchAuthenticatedImage, isAuthenticatedImagePath } from "../api";

interface AuthenticatedImageProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> {
  src: string;
  linkProps?: Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href" | "children">;
}

export function AuthenticatedImage({ src, linkProps, ...imageProps }: AuthenticatedImageProps) {
  const [resolvedSrc, setResolvedSrc] = useState<string | undefined>(() => isAuthenticatedImagePath(src) ? undefined : src);

  useEffect(() => {
    if (!isAuthenticatedImagePath(src)) {
      setResolvedSrc(src);
      return;
    }

    const controller = new AbortController();
    let objectUrl: string | undefined;
    setResolvedSrc(undefined);

    void fetchAuthenticatedImage(src, controller.signal)
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(blob);
        setResolvedSrc(objectUrl);
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setResolvedSrc(undefined);
      });

    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [src]);

  const image = <img {...imageProps} src={resolvedSrc} />;
  if (!linkProps) return image;

  return (
    <a {...linkProps} href={resolvedSrc}>
      {image}
    </a>
  );
}
