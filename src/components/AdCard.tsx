import { useEffect } from "react";

import { adImageUrl } from "../api/backend";
import type { AdCreative } from "../lib/ads";

type AdCardProps = {
  creative: AdCreative | null;
  onImpression: (creativeId: string) => void;
  onClick: (creativeId: string) => void;
};

/**
 * A static image/text ad card.
 *
 * The 「广告」 badge and advertiser attribution are mandatory, not decorative: Chinese advertising
 * regulation requires ads to be identifiable as such. The image is served from the local cache via
 * the `clip-media` protocol, so this component issues no external request.
 */
export function AdCard({ creative, onImpression, onClick }: AdCardProps) {
  const creativeId = creative?.creativeId ?? null;

  useEffect(() => {
    if (creativeId !== null) onImpression(creativeId);
  }, [creativeId, onImpression]);

  if (creative === null) return null;

  return (
    <div className="ad-card">
      <button
        aria-label={`广告：${creative.title}（${creative.advertiserName}）`}
        className="ad-card-button"
        type="button"
        onClick={() => onClick(creative.creativeId)}
      >
        <img
          alt=""
          className="ad-card-image"
          loading="lazy"
          src={adImageUrl(creative.imagePath)}
        />
        <span className="ad-card-text">
          <strong className="ad-card-title">{creative.title}</strong>
          {creative.body ? (
            <span className="ad-card-body">{creative.body}</span>
          ) : null}
        </span>
      </button>
      <span className="ad-card-meta">
        <em className="ad-card-badge">广告</em>
        <span className="ad-card-advertiser">{creative.advertiserName}</span>
      </span>
    </div>
  );
}
