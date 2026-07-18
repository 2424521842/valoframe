import { useCallback, useEffect, useRef, useState } from "react";
import { commandErrorMessage, getLibraryFacets } from "../api/backend";
import type { LibraryFacets } from "../types";

export type LibraryFacetsController = {
  facets: LibraryFacets | null;
  error: string | null;
  isLoading: boolean;
  refresh: () => Promise<boolean>;
};

export function useLibraryFacetsController(): LibraryFacetsController {
  const mountedRef = useRef(false);
  const didRequestInitialFacetsRef = useRef(false);
  const requestTokenRef = useRef(0);
  const [facets, setFacets] = useState<LibraryFacets | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async (): Promise<boolean> => {
    const requestToken = requestTokenRef.current + 1;
    requestTokenRef.current = requestToken;
    if (mountedRef.current) {
      setIsLoading(true);
    }

    try {
      const nextFacets = await getLibraryFacets();
      if (!mountedRef.current || requestTokenRef.current !== requestToken) {
        return false;
      }
      setFacets(nextFacets);
      setError(null);
      return true;
    } catch (requestError) {
      if (!mountedRef.current || requestTokenRef.current !== requestToken) {
        return false;
      }
      setError(
        `统计加载失败：${commandErrorMessage(requestError)}；素材分页仍可使用。`,
      );
      return false;
    } finally {
      if (mountedRef.current && requestTokenRef.current === requestToken) {
        setIsLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    if (didRequestInitialFacetsRef.current) return;
    didRequestInitialFacetsRef.current = true;
    void refresh();
  }, [refresh]);

  return { facets, error, isLoading, refresh };
}
