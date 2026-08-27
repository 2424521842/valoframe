import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  listAdCreatives,
  recordAdClick,
  recordAdImpression,
  refreshAdCreatives,
} from "../api/backend";
import {
  activeCreatives,
  selectCreative,
  type AdCreative,
  type AdSlot,
} from "../lib/ads";

export type AdController = {
  /** Null whenever ads are disabled, the manifest is unavailable, or nothing is in flight. */
  creative: AdCreative | null;
  onImpression: (creativeId: string) => void;
  onClick: (creativeId: string) => Promise<void>;
};

export type UseAdControllerOptions = {
  slot: AdSlot;
  /** Distinct per mounted slot so two slots do not always show the same creative. */
  rotationSeed?: number;
};

export function useAdController(options: UseAdControllerOptions): AdController {
  const { slot, rotationSeed = 0 } = options;
  const [creatives, setCreatives] = useState<AdCreative[]>([]);
  const reportedImpressions = useRef(new Set<string>());

  useEffect(() => {
    let cancelled = false;
    setCreatives([]);
    void (async () => {
      // The backend owns the trusted endpoint and landing-host policy. Refresh failures are
      // expected offline and fail closed: stale cached campaigns are never displayed.
      try {
        await refreshAdCreatives();
      } catch {
        return;
      }
      try {
        const cached = await listAdCreatives();
        if (!cancelled) setCreatives(cached);
      } catch {
        if (!cancelled) setCreatives([]);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const creative = useMemo(() => {
    return selectCreative(activeCreatives(creatives), rotationSeed);
  }, [creatives, rotationSeed]);

  const onImpression = useCallback(
    (creativeId: string) => {
      // One impression per creative per mount; the backend aggregates per day anyway.
      if (reportedImpressions.current.has(creativeId)) return;
      reportedImpressions.current.add(creativeId);
      void recordAdImpression(creativeId, slot).catch(() => undefined);
    },
    [slot],
  );

  const onClick = useCallback(
    async (creativeId: string) => {
      try {
        await recordAdClick(creativeId, slot);
      } catch {
        // The backend refuses to open anything it cannot validate; nothing useful to show here.
      }
    },
    [slot],
  );

  return { creative, onImpression, onClick };
}
