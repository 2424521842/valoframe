import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Tag } from "../../src/types";

const mocks = vi.hoisted(() => ({
  createTag: vi.fn(),
  deleteTag: vi.fn(),
  listTags: vi.fn(),
  updateTag: vi.fn(),
}));

vi.mock("../../src/api/backend", () => ({
  commandErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  createTag: mocks.createTag,
  deleteTag: mocks.deleteTag,
  listTags: mocks.listTags,
  updateTag: mocks.updateTag,
}));

import { useTagController } from "../../src/hooks/useTagController";

const tagA: Tag = { id: "a", label: "标签 A", color: "blue" };
const tagB: Tag = { id: "b", label: "标签 B", color: "teal" };

describe("useTagController", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listTags.mockResolvedValue([tagA]);
    mocks.createTag.mockResolvedValue(tagB);
    mocks.updateTag.mockResolvedValue({ ...tagB, label: "标签 B2", color: "gold" });
    mocks.deleteTag.mockResolvedValue(undefined);
  });

  it("loads once under StrictMode", async () => {
    const controller = renderController(true);

    await waitFor(() => expect(controller.result.current.isLoading).toBe(false));
    expect(mocks.listTags).toHaveBeenCalledTimes(1);
    expect(controller.result.current.tags).toEqual([tagA]);
    expect(controller.result.current.error).toBeNull();
  });

  it("lets the newest refresh win and keeps last-known-good tags on failure", async () => {
    const oldRequest = deferred<Tag[]>();
    const newRequest = deferred<Tag[]>();
    mocks.listTags
      .mockResolvedValueOnce([tagA])
      .mockReturnValueOnce(oldRequest.promise)
      .mockReturnValueOnce(newRequest.promise)
      .mockRejectedValueOnce(new Error("offline"));
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.tags).toEqual([tagA]));

    let oldPromise!: Promise<boolean>;
    let newPromise!: Promise<boolean>;
    act(() => {
      oldPromise = controller.result.current.refresh();
      newPromise = controller.result.current.refresh();
    });
    await act(async () => {
      newRequest.resolve([tagB]);
      expect(await newPromise).toBe(true);
    });
    await act(async () => {
      oldRequest.resolve([tagA]);
      expect(await oldPromise).toBe(false);
    });
    expect(controller.result.current.tags).toEqual([tagB]);

    await act(async () => {
      expect(await controller.result.current.refresh()).toBe(false);
    });
    expect(controller.result.current.tags).toEqual([tagB]);
    expect(controller.result.current.error).toBe("offline");
    expect(controller.activity).toHaveBeenCalledWith("标签加载失败：offline");
  });

  it("creates, updates, and deletes tags while coordinating facets and consumers", async () => {
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.tags).toEqual([tagA]));

    await act(async () => {
      expect(await controller.result.current.create("标签 B", "teal")).toEqual(tagB);
    });
    expect(controller.result.current.tags).toEqual([tagA, tagB]);
    expect(controller.activity).toHaveBeenCalledWith("已创建标签：标签 B");

    await act(async () => {
      expect(await controller.result.current.update("b", "标签 B2", "gold"))
        .toEqual({ ...tagB, label: "标签 B2", color: "gold" });
    });
    expect(controller.result.current.tags[1]).toMatchObject({
      id: "b",
      label: "标签 B2",
      color: "gold",
    });

    await act(async () => {
      expect(await controller.result.current.remove("b")).toBe(true);
    });
    expect(controller.result.current.tags).toEqual([tagA]);
    expect(controller.onTagDeleted).toHaveBeenCalledWith("b");
    expect(controller.refreshFacets).toHaveBeenCalledTimes(3);
    expect(controller.activity).toHaveBeenCalledWith("已删除标签：标签 B2");
  });

  it("does not patch local tags when CRUD commands fail", async () => {
    mocks.createTag.mockRejectedValueOnce(new Error("create failed"));
    mocks.updateTag.mockRejectedValueOnce(new Error("update failed"));
    mocks.deleteTag.mockRejectedValueOnce(new Error("delete failed"));
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.tags).toEqual([tagA]));

    await act(async () => {
      expect(await controller.result.current.create("B")).toBeNull();
      expect(await controller.result.current.update("a", "A2", "red")).toBeNull();
      expect(await controller.result.current.remove("a")).toBe(false);
    });
    expect(controller.result.current.tags).toEqual([tagA]);
    expect(controller.refreshFacets).not.toHaveBeenCalled();
    expect(controller.onTagDeleted).not.toHaveBeenCalled();
  });

  it("a completed mutation supersedes an older list refresh without leaving loading stuck", async () => {
    const staleRefresh = deferred<Tag[]>();
    mocks.listTags
      .mockResolvedValueOnce([tagA])
      .mockReturnValueOnce(staleRefresh.promise);
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.tags).toEqual([tagA]));

    let refreshPromise!: Promise<boolean>;
    act(() => {
      refreshPromise = controller.result.current.refresh();
    });
    expect(controller.result.current.isLoading).toBe(true);
    await act(async () => {
      await controller.result.current.create("标签 B", "teal");
    });
    expect(controller.result.current.isLoading).toBe(false);
    expect(controller.result.current.tags).toEqual([tagA, tagB]);

    await act(async () => {
      staleRefresh.resolve([tagA]);
      expect(await refreshPromise).toBe(false);
    });
    expect(controller.result.current.tags).toEqual([tagA, tagB]);
  });

  it("does not write through consumer callbacks after unmount", async () => {
    const pendingCreate = deferred<Tag>();
    mocks.createTag.mockReturnValueOnce(pendingCreate.promise);
    const controller = renderController();
    await waitFor(() => expect(controller.result.current.tags).toEqual([tagA]));
    const promise = controller.result.current.create("标签 B", "teal");

    controller.unmount();
    pendingCreate.resolve(tagB);
    await expect(promise).resolves.toEqual(tagB);
    expect(controller.refreshFacets).not.toHaveBeenCalled();
    expect(controller.onTagDeleted).not.toHaveBeenCalled();
    expect(controller.activity).not.toHaveBeenCalled();
  });
});

function renderController(reactStrictMode = false) {
  const activity = vi.fn();
  const refreshFacets = vi.fn(async () => true);
  const onTagDeleted = vi.fn();
  const controller = renderHook(
    () => useTagController({
      onActivityMessage: activity,
      refreshFacets,
      onTagDeleted,
    }),
    { reactStrictMode },
  );
  return { ...controller, activity, refreshFacets, onTagDeleted };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}
