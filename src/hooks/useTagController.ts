import { useCallback, useEffect, useRef, useState } from "react";
import {
  commandErrorMessage,
  createTag,
  deleteTag,
  listTags,
  updateTag,
} from "../api/backend";
import type { Tag, TagColor } from "../types";

export type UseTagControllerOptions = {
  onActivityMessage: (message: string) => void;
  refreshFacets: () => Promise<boolean>;
  onTagDeleted: (tagId: string) => void;
};

export type TagController = {
  tags: Tag[];
  isLoading: boolean;
  error: string | null;
  refresh: () => Promise<boolean>;
  create: (name: string, color?: TagColor) => Promise<Tag | null>;
  update: (tagId: string, name: string, color: TagColor) => Promise<Tag | null>;
  remove: (tagId: string) => Promise<boolean>;
};

export function useTagController({
  onActivityMessage,
  refreshFacets,
  onTagDeleted,
}: UseTagControllerOptions): TagController {
  const mountedRef = useRef(false);
  const didRequestInitialTagsRef = useRef(false);
  const requestTokenRef = useRef(0);
  const activityRef = useRef(onActivityMessage);
  const refreshFacetsRef = useRef(refreshFacets);
  const onTagDeletedRef = useRef(onTagDeleted);
  const tagsRef = useRef<Tag[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  activityRef.current = onActivityMessage;
  refreshFacetsRef.current = refreshFacets;
  onTagDeletedRef.current = onTagDeleted;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const replaceTags = useCallback((nextTags: Tag[]) => {
    tagsRef.current = nextTags;
    if (mountedRef.current) setTags(nextTags);
  }, []);

  const updateTags = useCallback((updater: (current: readonly Tag[]) => Tag[]) => {
    replaceTags(updater(tagsRef.current));
  }, [replaceTags]);

  const supersedeRefresh = useCallback(() => {
    requestTokenRef.current += 1;
    if (mountedRef.current) {
      setIsLoading(false);
      setError(null);
    }
  }, []);

  const refresh = useCallback(async (): Promise<boolean> => {
    const requestToken = requestTokenRef.current + 1;
    requestTokenRef.current = requestToken;
    if (mountedRef.current) setIsLoading(true);
    try {
      const nextTags = await listTags();
      if (!mountedRef.current || requestTokenRef.current !== requestToken) {
        return false;
      }
      replaceTags(nextTags);
      setError(null);
      return true;
    } catch (requestError) {
      if (!mountedRef.current || requestTokenRef.current !== requestToken) {
        return false;
      }
      const message = commandErrorMessage(requestError);
      setError(message);
      activityRef.current(`标签加载失败：${message}`);
      return false;
    } finally {
      if (mountedRef.current && requestTokenRef.current === requestToken) {
        setIsLoading(false);
      }
    }
  }, [replaceTags]);

  useEffect(() => {
    if (didRequestInitialTagsRef.current) return;
    didRequestInitialTagsRef.current = true;
    void refresh();
  }, [refresh]);

  const create = useCallback(async (
    name: string,
    color: TagColor = "blue",
  ): Promise<Tag | null> => {
    try {
      const tag = await createTag(name, color);
      if (!mountedRef.current) return tag;
      supersedeRefresh();
      updateTags((current) =>
        current.some((candidate) => candidate.id === tag.id)
          ? current.map((candidate) => candidate.id === tag.id ? tag : candidate)
          : [...current, tag],
      );
      await refreshFacetsRef.current();
      if (mountedRef.current) activityRef.current(`已创建标签：${tag.label}`);
      return tag;
    } catch (requestError) {
      if (mountedRef.current) {
        activityRef.current(`标签创建失败：${commandErrorMessage(requestError)}`);
      }
      return null;
    }
  }, [supersedeRefresh, updateTags]);

  const update = useCallback(async (
    tagId: string,
    name: string,
    color: TagColor,
  ): Promise<Tag | null> => {
    try {
      const tag = await updateTag(tagId, name, color);
      if (!mountedRef.current) return tag;
      supersedeRefresh();
      updateTags((current) =>
        current.map((candidate) => candidate.id === tag.id ? tag : candidate),
      );
      await refreshFacetsRef.current();
      if (mountedRef.current) activityRef.current(`已更新标签：${tag.label}`);
      return tag;
    } catch (requestError) {
      if (mountedRef.current) {
        activityRef.current(`标签更新失败：${commandErrorMessage(requestError)}`);
      }
      return null;
    }
  }, [supersedeRefresh, updateTags]);

  const remove = useCallback(async (tagId: string): Promise<boolean> => {
    const tag = tagsRef.current.find((candidate) => candidate.id === tagId);
    try {
      await deleteTag(tagId);
      if (!mountedRef.current) return true;
      supersedeRefresh();
      updateTags((current) => current.filter((candidate) => candidate.id !== tagId));
      onTagDeletedRef.current(tagId);
      await refreshFacetsRef.current();
      if (mountedRef.current) {
        activityRef.current(`已删除标签：${tag?.label ?? "自定义标签"}`);
      }
      return true;
    } catch (requestError) {
      if (mountedRef.current) {
        activityRef.current(`标签删除失败：${commandErrorMessage(requestError)}`);
      }
      return false;
    }
  }, [supersedeRefresh, updateTags]);

  return {
    tags,
    isLoading,
    error,
    refresh,
    create,
    update,
    remove,
  };
}
