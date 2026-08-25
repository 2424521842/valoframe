import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  listAdCreatives,
  recordAdClick,
  recordAdImpression,
  refreshAdCreatives,
} from "../api/backend";
import {
  activeCreatives,
  parseAllowedHosts,
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
  enabled: boolean;
  manifestEndpoint: string;
  allowedHosts: string;
  slot: AdSlot;
  /** Distinct per mounted slot so two slots do not always show the same creative. */
  rotationSeed?: number;
};

export function useAdController(options: UseAdControllerOptions): AdController {
  const { enabled, manifestEndpoint, allowedHosts, slot, rotationSeed = 0 } = options;
  const [creatives, setCreatives] = useState<AdCreative[]>([]);
  const reportedImpressions = useRef(new Set<string>());

  useEffect(() => {
    if (!enabled || manifestEndpoint.trim() === "") {
      setCreatives([]);
      return;
    }

    let cancelled = false;
    void (async () => {
      // A refresh failure is expected offline and must never surface as an error: the slot just
      // stays on whatever is already cached, or empty.
      try {
        await refreshAdCreatives(manifestEndpoint);
      } catch {
        // Intentionally ignored; fall through to the cached list.
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
  }, [enabled, manifestEndpoint]);

  const creative = useMemo(() => {
    if (!enabled) return null;
    return selectCreative(activeCreatives(creatives), rotationSeed);
  }, [creatives, enabled, rotationSeed]);

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
      const hosts = parseAllowedHosts(allowedHosts);
      if (hosts.length === 0) return;
      try {
        await recordAdClick(creativeId, slot, hosts);
      } catch {
        // The backend refuses to open anything it cannot validate; nothing useful to show here.
      }
    },
    [allowedHosts, slot],
  );

  return { creative, onImpression, onClick };
}
