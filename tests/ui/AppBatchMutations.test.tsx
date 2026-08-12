import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockClips, mockSourceDirs, mockTags } from "../../src/data/mockData";
import type { AppUpdaterController } from "../../src/hooks/useAppUpdaterController";
import type { BatchMutationResult, Clip, ClipListQuery, ClipPage, ClipSummary } from "../../src/types";
import { libraryFacets } from "./libraryFacetFixtures";

const mocks = vi.hoisted(() => ({
  addTagToClips: vi.fn(),
  deleteClipsPermanently: vi.fn(),
  exportClips: vi.fn(),
  getLibraryFacets: vi.fn(),
  listClips: vi.fn(),
  listClipPage: vi.fn(),
  listSources: vi.fn(),
  listTags: vi.fn(),
  removeClipsFromIndex: vi.fn(),
  removeTagFromClips: vi.fn(),
  selectExportDirectory: vi.fn(),
  setClipsFavorite: vi.fn(),
  setClipsTrashed: vi.fn(),
  appUpdater: {
    runtimeInfo: {
      currentVersion: "0.2.1",
      channel: "stable",
      endpoint: "https://github.com/2424521842/valoframe/releases/latest/download/latest.json",
      configured: true,
    },
    runtimeStatus: "ready",
    runtimeError: null,
    phase: "idle",
    update: null,
    progress: { downloadedBytes: 0, totalBytes: null },
    message: "更新检查尚未运行",
    error: null,
    canCheck: true,
    canDownload: false,
    canCancelDownload: false,
    canInstall: false,
    refreshRuntimeInfo: vi.fn(async () => undefined),
    checkManually: vi.fn(async () => undefined),
    download: vi.fn(async () => undefined),
    cancelDownload: vi.fn(async () => undefined),
    installAndRestart: vi.fn(async () => undefined),
  } as AppUpdaterController,
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tauri-apps/api/core")>();
  return {
    ...actual,
    isTauri: () => true,
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.selectExportDirectory,
}));

vi.mock("../../src/hooks/useAppUpdaterController", () => ({
  useAppUpdaterController: () => mocks.appUpdater,
}));

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    addTagToClips: mocks.addTagToClips,
    deleteClipsPermanently: mocks.deleteClipsPermanently,
    exportClips: mocks.exportClips,
    getLibraryFacets: mocks.getLibraryFacets,
    listClips: mocks.listClips,
    listClipPage: mocks.listClipPage,
    listSources: mocks.listSources,
    listTags: mocks.listTags,
    removeClipsFromIndex: mocks.removeClipsFromIndex,
    removeTagFromClips: mocks.removeTagFromClips,
    setClipsFavorite: mocks.setClipsFavorite,
    setClipsTrashed: mocks.setClipsTrashed,
    listenToScanProgress: vi.fn(async () => () => undefined),
  };
});

import App from "../../src/App";

let fixtureClips: Clip[];

describe("production batch mutation flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetAppUpdater();
    fixtureClips = mockClips.slice(0, 2).map(cloneClip);
    mocks.listClipPage.mockImplementation(async (query: ClipListQuery) => pageForQuery(fixtureClips, query));
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      totalCount: fixtureClips.length,
      activeCount: fixtureClips.length,
    }));
    mocks.listSources.mockResolvedValue(mockSourceDirs.map((source) => ({ ...source })));
    mocks.listTags.mockResolvedValue(mockTags.map((tag) => ({ ...tag })));
    mocks.selectExportDirectory.mockResolvedValue("D:\\Exports");
    mocks.exportClips.mockImplementation(async (clipIds: string[], destinationDir: string) => ({
      requested: new Set(clipIds).size,
      exported: new Set(clipIds).size,
      failed: 0,
      destinationDir,
      exportedIds: [...new Set(clipIds)],
      missingIds: [],
      missingFileIds: [],
      exports: [...new Set(clipIds)].map((clipId) => ({
        clipId,
        fileName: `${clipId}.mp4`,
        destinationPath: `${destinationDir}\\${clipId}.mp4`,
        bytesCopied: 1024,
      })),
      failures: [],
    }));
    mocks.setClipsFavorite.mockImplementation(
      async (clipIds: string[], isFavorite: boolean) => mutationResult(
        clipIds,
        fixtureClips
          .filter((clip) => clipIds.includes(clip.id))
          .map((clip) => ({ ...cloneClip(clip), isFavorite })),
      ),
    );
    mocks.addTagToClips.mockImplementation(
      async (clipIds: string[], tagId: string) => mutationResult(
        clipIds,
        fixtureClips
          .filter((clip) => clipIds.includes(clip.id))
          .map((clip) => ({
            ...cloneClip(clip),
            tags: clip.tags.includes(tagId) ? [...clip.tags] : [...clip.tags, tagId],
          })),
      ),
    );
    mocks.removeTagFromClips.mockImplementation(
      async (clipIds: string[], tagId: string) => mutationResult(
        clipIds,
        fixtureClips
          .filter((clip) => clipIds.includes(clip.id))
          .map((clip) => ({
            ...cloneClip(clip),
            tags: clip.tags.filter((candidate) => candidate !== tagId),
          })),
      ),
    );
    mocks.setClipsTrashed.mockImplementation(
      async (clipIds: string[], isTrashed: boolean) => {
        fixtureClips = fixtureClips.map((clip) =>
          clipIds.includes(clip.id)
            ? { ...cloneClip(clip), fileStatus: isTrashed ? "trashed" : "available" }
            : clip,
        );
        return mutationResult(
          clipIds,
          fixtureClips.filter((clip) => clipIds.includes(clip.id)).map(cloneClip),
        );
      },
    );
    mocks.deleteClipsPermanently.mockImplementation(async (clipIds: string[]) => {
      const uniqueIds = [...new Set(clipIds)];
      const deletedIds = fixtureClips
        .filter((clip) => uniqueIds.includes(clip.id) && clip.fileStatus === "trashed")
        .map((clip) => clip.id);
      const missingIds = uniqueIds.filter((clipId) =>
        !fixtureClips.some((clip) => clip.id === clipId),
      );
      fixtureClips = fixtureClips.filter((clip) => !deletedIds.includes(clip.id));
      return {
        requested: uniqueIds.length,
        deletedIds,
        missingIds,
        pendingIds: [],
        blocked: [],
        failures: [],
      };
    });
    mocks.removeClipsFromIndex.mockImplementation(async (clipIds: string[]) => {
      const uniqueIds = [...new Set(clipIds)];
      const removedIds = fixtureClips
        .filter((clip) => uniqueIds.includes(clip.id) && clip.fileStatus === "missing")
        .map((clip) => clip.id);
      const missingIds = uniqueIds.filter((clipId) =>
        !fixtureClips.some((clip) => clip.id === clipId),
      );
      const blocked = fixtureClips
        .filter((clip) => uniqueIds.includes(clip.id) && !removedIds.includes(clip.id))
        .map((clip) => ({
          clipId: clip.id,
          code: "index-removal-not-eligible",
          message: "素材仍可用",
        }));
      fixtureClips = fixtureClips.filter((clip) => !removedIds.includes(clip.id));
      return {
        requested: uniqueIds.length,
        removedIds,
        missingIds,
        blocked,
        failures: [],
      };
    });
  });

  it("favorites a multi-selection once and merges the returned clips without refreshing", async () => {
    const user = userEvent.setup();
    render(<App />);
    await selectAllVisible(user);
    const loadsBeforeMutation = mocks.listClipPage.mock.calls.length;
    const facetsBeforeMutation = mocks.getLibraryFacets.mock.calls.length;

    await user.click(within(batchToolbar()).getByRole("button", { name: "取消收藏" }));

    await waitFor(() => expect(mocks.setClipsFavorite).toHaveBeenCalledTimes(1));
    expect(mocks.setClipsFavorite).toHaveBeenCalledWith(
      fixtureClips.map((clip) => clip.id),
      false,
    );
    await waitFor(() => {
      expect(favoriteButton(fixtureClips[0].id)).toHaveAttribute("aria-pressed", "false");
      expect(favoriteButton(fixtureClips[1].id)).toHaveAttribute("aria-pressed", "false");
    });
    expect(mocks.listClipPage).toHaveBeenCalledTimes(loadsBeforeMutation);
    expect(mocks.getLibraryFacets.mock.calls.length).toBeGreaterThan(facetsBeforeMutation);
    expect(mocks.listClips).not.toHaveBeenCalled();
  });

  it("loads and selects every filtered result with one select-all click", async () => {
    const user = userEvent.setup();
    fixtureClips = Array.from({ length: 52 }, (_, index) => ({
      ...cloneClip(mockClips[0]),
      id: String(index + 1),
      fileName: `clip-${index + 1}.mp4`,
      filePath: `D:\\Highlights\\clip-${index + 1}.mp4`,
    }));
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      totalCount: fixtureClips.length,
      activeCount: fixtureClips.length,
    }));
    render(<App />);

    await user.click(await screen.findByRole("checkbox", {
      name: "选择全部 52 条结果",
    }));

    await waitFor(() => expect(batchToolbar()).toHaveTextContent("已选择 52 条素材"));
    expect(mocks.listClipPage.mock.calls.map(([query]) => query.offset)).toEqual([0, 50]);
    expect(screen.getByRole("checkbox", { name: "取消选择全部结果" })).toBeChecked();
  });

  it("keeps cross-page cleanup eligibility and confirms every index state that will be lost", async () => {
    const user = userEvent.setup();
    fixtureClips = Array.from({ length: 52 }, (_, index) => ({
      ...cloneClip(mockClips[0]),
      id: String(index + 1),
      fileName: `clip-${index + 1}.mp4`,
      filePath: `D:\\Highlights\\clip-${index + 1}.mp4`,
      fileStatus: index === 51 ? "missing" : "available",
    }));
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      totalCount: fixtureClips.length,
      activeCount: fixtureClips.length,
    }));
    render(<App />);

    await user.click(await screen.findByRole(
      "checkbox",
      { name: "选择全部 52 条结果" },
      { timeout: 5_000 },
    ));
    await waitFor(() => expect(batchToolbar()).toHaveTextContent("已选择 52 条素材"));
    const cleanup = within(batchToolbar()).getByRole("button", {
      name: "仅移除失联索引 (1)",
    });
    await user.click(cleanup);

    const confirmation = await screen.findByRole("alertdialog");
    for (const state of ["收藏", "标签", "备注", "评审决定", "缩略图状态", "结构化元数据"]) {
      expect(confirmation).toHaveTextContent(state);
    }
    expect(confirmation).toHaveTextContent("绝不会删除、移动或修改磁盘上的视频文件");
    await user.click(within(confirmation).getByRole("button", { name: "仅移除索引" }));

    await waitFor(() => expect(mocks.removeClipsFromIndex).toHaveBeenCalledTimes(1));
    expect(mocks.removeClipsFromIndex).toHaveBeenCalledWith(["52"]);
    expect(await screen.findByText(/已从索引移除 1 条素材/)).toBeVisible();
    await waitFor(() => expect(batchToolbar()).toHaveTextContent("已选择 51 条素材"));
  });

  it("removes successful index rows locally and keeps blocked rows in the dialog for retry", async () => {
    const user = userEvent.setup();
    fixtureClips = fixtureClips.map((clip) => ({ ...cloneClip(clip), fileStatus: "missing" }));
    const [removedClip, blockedClip] = fixtureClips;
    mocks.removeClipsFromIndex.mockResolvedValueOnce({
      requested: 2,
      removedIds: [removedClip.id],
      missingIds: [],
      blocked: [{
        clipId: blockedClip.id,
        code: "delete-pending",
        message: "素材已进入永久删除队列",
      }],
      failures: [],
    });
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", {
      name: "仅移除失联索引 (2)",
    }));
    let confirmation = await screen.findByRole("alertdialog");
    await user.click(within(confirmation).getByRole("button", { name: "仅移除索引" }));

    await waitFor(() => expect(mocks.removeClipsFromIndex).toHaveBeenCalledTimes(1));
    confirmation = await screen.findByRole("alertdialog");
    expect(confirmation).toHaveTextContent("永久移除 1 条索引记录");
    expect(within(confirmation).getByRole("alert")).toHaveTextContent(
      "本次成功移除 1 条；索引已不存在 0 条；阻断 1 条；失败 0 条",
    );
    expect(within(confirmation).getByRole("alert")).toHaveTextContent(
      "素材已进入永久删除队列",
    );
    expect(batchToolbar()).toHaveTextContent("已选择 1 条素材");

    await user.click(within(confirmation).getByRole("button", { name: "仅移除索引" }));
    await waitFor(() => expect(mocks.removeClipsFromIndex).toHaveBeenCalledTimes(2));
    expect(mocks.removeClipsFromIndex).toHaveBeenNthCalledWith(2, [blockedClip.id]);
  });

  it("keeps an index-only removal failure inside the modal as a live retryable error", async () => {
    const user = userEvent.setup();
    fixtureClips = fixtureClips.map((clip) => ({ ...cloneClip(clip), fileStatus: "missing" }));
    mocks.removeClipsFromIndex.mockRejectedValueOnce(new Error("database locked"));
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", {
      name: "仅移除失联索引 (2)",
    }));
    const confirmation = await screen.findByRole("alertdialog");
    await user.click(within(confirmation).getByRole("button", { name: "仅移除索引" }));

    expect(await within(confirmation).findByRole("alert")).toHaveTextContent(
      "仅移除索引失败：本次未移除任何记录，请重试",
    );
    expect(within(confirmation).getByRole("button", { name: "仅移除索引" })).toBeEnabled();
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");

    await user.click(within(confirmation).getByRole("button", { name: "仅移除索引" }));
    await waitFor(() => expect(mocks.removeClipsFromIndex).toHaveBeenCalledTimes(2));
  });

  it("applies a batch tag once and reflects the new aggregate selection state", async () => {
    const user = userEvent.setup();
    render(<App />);
    await selectAllVisible(user);
    const facetsBeforeMutation = mocks.getLibraryFacets.mock.calls.length;
    await user.click(within(batchToolbar()).getByRole("button", { name: "自定义标签" }));

    await user.click(await screen.findByRole("checkbox", { name: "添加残局标签" }));

    await waitFor(() => expect(mocks.addTagToClips).toHaveBeenCalledTimes(1));
    expect(mocks.addTagToClips).toHaveBeenCalledWith(
      fixtureClips.map((clip) => clip.id),
      "clutch",
    );
    expect(await screen.findByRole("checkbox", { name: "移除残局标签" })).toBeChecked();
    expect(mocks.getLibraryFacets.mock.calls.length).toBeGreaterThan(facetsBeforeMutation);
  });

  it("exports the selected videos to a chosen folder and keeps the selection available", async () => {
    const user = userEvent.setup();
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "导出所选" }));

    await waitFor(() => expect(mocks.selectExportDirectory).toHaveBeenCalledTimes(1));
    expect(mocks.selectExportDirectory).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: "选择导出文件夹",
    });
    await waitFor(() => expect(mocks.exportClips).toHaveBeenCalledTimes(1));
    expect(mocks.exportClips).toHaveBeenCalledWith(
      fixtureClips.map((clip) => clip.id),
      "D:\\Exports",
    );
    expect(await screen.findByText(/已导出 2 条素材到 D:\\Exports/)).toBeVisible();
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");
  });

  it("does not export when folder selection is cancelled", async () => {
    const user = userEvent.setup();
    mocks.selectExportDirectory.mockResolvedValueOnce(null);
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "导出所选" }));

    expect(await screen.findByText(/已取消导出/)).toBeVisible();
    expect(mocks.exportClips).not.toHaveBeenCalled();
    expect(within(batchToolbar()).getByRole("button", { name: "导出所选" })).toBeEnabled();
  });

  it("reports a partial export without clearing the selected videos", async () => {
    const user = userEvent.setup();
    const [exportedClip, failedClip] = fixtureClips;
    mocks.exportClips.mockResolvedValueOnce({
      requested: 2,
      exported: 1,
      failed: 1,
      destinationDir: "D:\\Exports",
      exportedIds: [exportedClip.id],
      missingIds: [],
      missingFileIds: [failedClip.id],
      exports: [{
        clipId: exportedClip.id,
        fileName: exportedClip.fileName,
        destinationPath: `D:\\Exports\\${exportedClip.fileName}`,
        bytesCopied: exportedClip.sizeBytes,
      }],
      failures: [{
        clipId: failedClip.id,
        code: "source-file-missing",
        message: "源视频文件不存在",
      }],
    });
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "导出所选" }));

    expect(await screen.findByText(/导出部分完成：成功 1\/2 条，失败 1 条/)).toBeVisible();
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");
  });

  it("surfaces an export failure and leaves the selection ready to retry", async () => {
    const user = userEvent.setup();
    mocks.exportClips.mockRejectedValueOnce(new Error("磁盘空间不足"));
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "导出所选" }));

    expect(await screen.findByText(/导出失败：磁盘空间不足/)).toBeVisible();
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");
    expect(within(batchToolbar()).getByRole("button", { name: "导出所选" })).toBeEnabled();
  });

  it.each(["resolve", "reject"] as const)(
    "blocks update installation while an export is pending and clears the blocker after %s",
    async (settlement) => {
      const user = userEvent.setup();
      const exportTask = deferred<Awaited<ReturnType<typeof mocks.exportClips>>>();
      prepareDownloadedUpdater();
      mocks.exportClips.mockReturnValueOnce(exportTask.promise);
      render(<App />);
      await selectAllVisible(user);

      await user.click(within(batchToolbar()).getByRole("button", { name: "导出所选" }));
      await waitFor(() => expect(mocks.exportClips).toHaveBeenCalledTimes(1));
      await user.click(screen.getByRole("button", { name: /^设置/ }));

      expect(await screen.findByText("视频导出任务正在运行，请等待导出结束后再安装")).toBeVisible();
      expect(screen.getByRole("button", { name: "安装并重启" })).toBeDisabled();

      await act(async () => {
        if (settlement === "resolve") {
          exportTask.resolve({
            requested: fixtureClips.length,
            exported: fixtureClips.length,
            failed: 0,
            destinationDir: "D:\\Exports",
            exportedIds: fixtureClips.map((clip) => clip.id),
            missingIds: [],
            missingFileIds: [],
            exports: [],
            failures: [],
          });
        } else {
          exportTask.reject(new Error("导出设备已断开"));
        }
        await exportTask.promise.catch(() => undefined);
      });

      await waitFor(() => {
        expect(screen.queryByText(/视频导出任务正在运行/)).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "安装并重启" })).toBeEnabled();
      });
    },
  );

  it("recycles and restores a multi-selection with one backend call per action", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<App />);
    await selectAllVisible(user);
    const facetsBeforeRecycle = mocks.getLibraryFacets.mock.calls.length;
    await user.click(within(batchToolbar()).getByRole("button", { name: "移入回收站" }));
    const confirmation = await screen.findByRole("alertdialog");
    await user.click(within(confirmation).getByRole("button", { name: "移入回收站" }));
    await waitFor(() => expect(mocks.setClipsTrashed).toHaveBeenCalledTimes(1));
    expect(mocks.setClipsTrashed).toHaveBeenLastCalledWith(
      fixtureClips.map((clip) => clip.id),
      true,
    );
    expect(mocks.getLibraryFacets.mock.calls.length).toBeGreaterThan(facetsBeforeRecycle);

    unmount();
    vi.clearAllMocks();
    fixtureClips = fixtureClips.map((clip) => ({ ...cloneClip(clip), fileStatus: "trashed" }));
    mocks.listClipPage.mockImplementation(async (query: ClipListQuery) => pageForQuery(fixtureClips, query));
    mocks.listSources.mockResolvedValue(mockSourceDirs.map((source) => ({ ...source })));
    mocks.listTags.mockResolvedValue(mockTags.map((tag) => ({ ...tag })));
    mocks.setClipsTrashed.mockImplementation(
      async (clipIds: string[], isTrashed: boolean) => mutationResult(
        clipIds,
        fixtureClips.map((clip) => ({
          ...cloneClip(clip),
          fileStatus: isTrashed ? "trashed" : "available",
        })),
      ),
    );

    render(<App />);
    await user.click(await screen.findByRole("button", { name: /回收站/ }));
    await selectAllVisible(user);
    const facetsBeforeRestore = mocks.getLibraryFacets.mock.calls.length;
    await user.click(within(batchToolbar()).getByRole("button", { name: "恢复" }));

    await waitFor(() => expect(mocks.setClipsTrashed).toHaveBeenCalledTimes(1));
    expect(mocks.setClipsTrashed).toHaveBeenLastCalledWith(
      fixtureClips.map((clip) => clip.id),
      false,
    );
    expect(mocks.getLibraryFacets.mock.calls.length).toBeGreaterThan(facetsBeforeRestore);
  });

  it("permanently deletes selected recycle-bin videos after an irreversible warning", async () => {
    const user = userEvent.setup();
    fixtureClips = fixtureClips.map((clip) => ({
      ...cloneClip(clip),
      fileStatus: "trashed",
    }));
    mocks.getLibraryFacets.mockResolvedValue(libraryFacets({
      totalCount: fixtureClips.length,
      activeCount: 0,
      trashedCount: fixtureClips.length,
    }));
    render(<App />);
    await user.click(await screen.findByRole("button", { name: /回收站/ }));
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "永久删除视频" }));
    const confirmation = await screen.findByRole("alertdialog");
    expect(confirmation).toHaveTextContent("此操作无法撤销");
    expect(confirmation).toHaveTextContent("文件不会进入系统回收站");
    await user.click(within(confirmation).getByRole("button", { name: "永久删除视频" }));

    await waitFor(() => expect(mocks.deleteClipsPermanently).toHaveBeenCalledTimes(1));
    expect(mocks.deleteClipsPermanently).toHaveBeenCalledWith(
      mockClips.slice(0, 2).map((clip) => clip.id),
    );
    expect(await screen.findByText(/已永久删除 2 条素材的本地视频和索引/)).toBeVisible();
  });

  it("keeps local state unchanged on failure and permits a full retry", async () => {
    const user = userEvent.setup();
    mocks.setClipsFavorite.mockRejectedValueOnce(new Error("database locked"));
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "取消收藏" }));
    expect(await screen.findByText(/批量收藏失败，当前批次未更新：database locked/)).toBeVisible();
    expect(favoriteButton(fixtureClips[0].id)).toHaveAttribute("aria-pressed", "true");
    expect(favoriteButton(fixtureClips[1].id)).toHaveAttribute("aria-pressed", "true");

    await user.click(within(batchToolbar()).getByRole("button", { name: "取消收藏" }));
    await waitFor(() => expect(mocks.setClipsFavorite).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(favoriteButton(fixtureClips[0].id)).toHaveAttribute("aria-pressed", "false");
      expect(favoriteButton(fixtureClips[1].id)).toHaveAttribute("aria-pressed", "false");
    });
  });

  it("blocks duplicate batch submissions while the first request is pending", async () => {
    const user = userEvent.setup();
    const request = deferred<BatchMutationResult>();
    mocks.setClipsFavorite.mockReturnValueOnce(request.promise);
    render(<App />);
    await selectAllVisible(user);
    const favorite = within(batchToolbar()).getByRole("button", { name: "取消收藏" });

    fireEvent.click(favorite);
    fireEvent.click(favorite);

    expect(mocks.setClipsFavorite).toHaveBeenCalledTimes(1);
    expect(favorite).toBeDisabled();
    request.resolve(mutationResult(
      fixtureClips.map((clip) => clip.id),
      fixtureClips.map((clip) => ({ ...cloneClip(clip), isFavorite: false })),
    ));
    await waitFor(() => expect(favoriteButton(fixtureClips[0].id)).toHaveAttribute("aria-pressed", "false"));
  });

  it("surfaces partial matching and does not present missing clips as successful", async () => {
    const user = userEvent.setup();
    const first = { ...cloneClip(fixtureClips[0]), isFavorite: false };
    mocks.setClipsFavorite.mockResolvedValueOnce({
      requested: 2,
      matched: 1,
      updated: 1,
      missingIds: [fixtureClips[1].id],
      clips: [first],
    });
    render(<App />);
    await selectAllVisible(user);

    await user.click(within(batchToolbar()).getByRole("button", { name: "取消收藏" }));

    expect(await screen.findByText(new RegExp(
      `收藏部分完成：匹配 1/2 条；未找到 ID：${fixtureClips[1].id}`,
    ))).toBeVisible();
    expect(favoriteButton(fixtureClips[0].id)).toHaveAttribute("aria-pressed", "false");
    expect(favoriteButton(fixtureClips[1].id)).toHaveAttribute("aria-pressed", "true");
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");
  });

  it("does not clear a newer selection when an older recycle request resolves", async () => {
    const user = userEvent.setup();
    const request = deferred<BatchMutationResult>();
    mocks.setClipsTrashed.mockReturnValueOnce(request.promise);
    render(<App />);
    await screen.findByRole("checkbox", { name: /选择全部 \d+ 条结果/ });
    await user.click(cardCheckbox(fixtureClips[0].id));

    await user.click(within(batchToolbar()).getByRole("button", { name: "移入回收站" }));
    const confirmation = await screen.findByRole("alertdialog");
    await user.click(within(confirmation).getByRole("button", { name: "移入回收站" }));
    expect(mocks.setClipsTrashed).toHaveBeenCalledWith([fixtureClips[0].id], true);

    fireEvent.click(cardCheckbox(fixtureClips[1].id));
    expect(batchToolbar()).toHaveTextContent("已选择 2 条素材");
    fixtureClips = fixtureClips.map((clip) =>
      clip.id === fixtureClips[0].id ? { ...cloneClip(clip), fileStatus: "trashed" } : clip,
    );
    request.resolve(mutationResult(
      [fixtureClips[0].id],
      [{ ...cloneClip(fixtureClips[0]), fileStatus: "trashed" }],
    ));

    await waitFor(() => expect(batchToolbar()).toHaveTextContent("已选择 1 条素材"));
    expect(cardCheckbox(fixtureClips[1].id)).toBeChecked();
  });
});

async function selectAllVisible(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole(
    "checkbox",
    { name: /选择全部 \d+ 条结果/ },
    { timeout: 5_000 },
  ));
  await screen.findByLabelText("批量操作");
}

function batchToolbar(): HTMLElement {
  return screen.getByLabelText("批量操作");
}

function card(clipId: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(`[data-clip-id="${clipId}"]`);
  if (!element) throw new Error(`clip card not found: ${clipId}`);
  return element;
}

function cardCheckbox(clipId: string): HTMLElement {
  return within(card(clipId)).getByRole("checkbox", { hidden: true });
}

function favoriteButton(clipId: string): HTMLElement {
  return within(card(clipId)).getByRole("button", { name: /收藏/ });
}

function cloneClip(clip: Clip): Clip {
  return {
    ...clip,
    tags: [...clip.tags],
    clipEvents: clip.clipEvents?.map((event) => ({ ...event })),
  };
}

function toSummary(clip: Clip): ClipSummary {
  const summary = { ...cloneClip(clip) };
  delete (summary as Partial<Clip>).note;
  delete (summary as Partial<Clip>).extractedText;
  delete (summary as Partial<Clip>).clipEvents;
  delete (summary as Partial<Clip>).eventCount;
  delete (summary as Partial<Clip>).roundLabel;
  delete (summary as Partial<Clip>).weaponName;
  return summary;
}

function pageForQuery(clips: Clip[], query: ClipListQuery): ClipPage {
  const filtered = clips.filter((clip) =>
    query.fileStatus ? clip.fileStatus === query.fileStatus : clip.fileStatus !== "trashed",
  );
  const offset = query.offset ?? 0;
  const limit = query.limit ?? 50;
  const items = filtered.slice(offset, offset + limit).map(toSummary);
  const nextOffset = offset + items.length;
  return {
    items,
    offset,
    limit,
    totalCount: filtered.length,
    hasMore: nextOffset < filtered.length,
    nextOffset: nextOffset < filtered.length ? nextOffset : null,
  };
}

function mutationResult(
  requestedIds: readonly string[],
  clips: Clip[],
  missingIds: string[] = [],
): BatchMutationResult {
  return {
    requested: new Set(requestedIds).size,
    matched: clips.length,
    updated: clips.length,
    missingIds,
    clips,
  };
}

function resetAppUpdater() {
  Object.assign(mocks.appUpdater, {
    runtimeInfo: {
      currentVersion: "0.2.1",
      channel: "stable",
      endpoint: "https://github.com/2424521842/valoframe/releases/latest/download/latest.json",
      configured: true,
    },
    runtimeStatus: "ready",
    runtimeError: null,
    phase: "idle",
    update: null,
    progress: { downloadedBytes: 0, totalBytes: null },
    message: "更新检查尚未运行",
    error: null,
    canCheck: true,
    canDownload: false,
    canCancelDownload: false,
    canInstall: false,
  });
}

function prepareDownloadedUpdater() {
  Object.assign(mocks.appUpdater, {
    phase: "downloaded",
    update: {
      currentVersion: "0.2.1",
      version: "0.2.2",
      notes: "自动更新集成测试",
      publishedAt: "2026-08-10T00:00:00Z",
    },
    message: "更新包已下载并通过签名验证，可以安装",
    error: null,
    canCheck: false,
    canDownload: false,
    canCancelDownload: false,
    canInstall: true,
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, reject, resolve };
}
