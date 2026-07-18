import { useEffect, useState, type ImgHTMLAttributes, type ReactNode } from "react";

type ThumbnailImageProps = Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> & {
  src: string | null;
  fallback: ReactNode;
};

/**
 * Keeps protocol failures local to the artwork that requested them. Any source
 * transition, including null followed by the same URL, clears the remembered
 * failure so a regenerated cache entry can be requested again.
 */
export function ThumbnailImage({
  src,
  fallback,
  onError,
  ...imageProps
}: ThumbnailImageProps) {
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  useEffect(() => {
    setFailedSrc(null);
  }, [src]);

  if (!src || failedSrc === src) {
    return fallback;
  }

  return (
    <img
      {...imageProps}
      src={src}
      onError={(event) => {
        setFailedSrc(src);
        onError?.(event);
      }}
    />
  );
}
