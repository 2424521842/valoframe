import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  discardFeedbackPackage,
  listenToFeedbackProgress,
  saveFeedbackPackage,
  submitFeedback,
} from "../../src/api/backend";
import { save } from "@tauri-apps/plugin-dialog";
import { FeedbackDialog } from "../../src/components/FeedbackDialog";
import { mockClips } from "../../src/data/mockData";
import type { FeedbackProgress, FeedbackSubmitResult } from "../../src/types";

vi.mock("../../src/api/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/api/backend")>();
  return {
    ...actual,
    discardFeedbackPackage: vi.fn(async () => undefined),
    listenToFeedbackProgress: vi.fn(async () => () => undefined),
    saveFeedbackPackage: vi.fn(async () => ({
      destinationPath: "C:\\reports\\package.zip",
      totalBytes: 1024,
    })),
    submitFeedback: vi.fn(),
  };
});

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

const submitFeedbackMock = vi.mocked(submitFeedback);
const saveFeedbackPackageMock = vi.mocked(saveFeedbackPackage);
const discardFeedbackPackageMock = vi.mocked(discardFeedbackPackage);
const listenMock = vi.mocked(listenToFeedbackProgress);
const saveDialogMock = vi.mocked(save);

const clip = { ...mockClips[0], id: "clip-a", fileStatus: "available", sizeBytes: 12_000_000 };

function uploadedResult(): FeedbackSubmitResult {
  return {
    reportId: "vhm-test",
    status: "uploaded",
    packagePath: null,
    suggestedFileName: "valoframe-feedback-1.zip",
    totalBytes: 4096,
    includedItems: ["诊断元数据（对局与素材信息）"],
    message: "问题反馈已上传，感谢你的帮助！",
    uploadError: null,
  };
}

function needsSaveResult(): FeedbackSubmitResult {
  return {
    reportId: "vhm-test",
    status: "needs-save",
    packagePath: "C:\\cache\\feedback\\vhm-test\\package.zip",
    suggestedFileName: "valoframe-feedback-1.zip",
    totalBytes: 4096,
    includedItems: ["诊断元数据（对局与素材信息）"],
    message: "诊断包已生成。",
    uploadError: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function renderDialog(endpoint = "") {
  render(
    <FeedbackDialog
      clip={clip}
      endpoint={endpoint}
      open
      onOpenChange={vi.fn()}
    />,
  );
}

describe("FeedbackDialog", () => {
  beforeEach(() => {
    submitFeedbackMock.mockReset();
    saveFeedbackPackageMock.mockReset();
    discardFeedbackPackageMock.mockReset();
    listenMock.mockReset();
    saveDialogMock.mockReset();
    saveFeedbackPackageMock.mockResolvedValue({
      destinationPath: "C:\\reports\\package.zip",
      totalBytes: 4096,
    });
  });

  it("shows the privacy scope and disables oversized video attachment", () => {
    renderDialog();

    expect(screen.getByRole("heading", { name: "反馈问题" })).toBeVisible();
    const privacy = screen.getByText(/不包含：/).closest(".feedback-privacy");
    expect(privacy).toHaveTextContent("OpenID");
    expect(screen.getByRole("checkbox", { name: /采样帧/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /完整视频/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: "生成诊断包" })).toBeDisabled();

    const oversizedClip = { ...clip, sizeBytes: 2 * 1024 * 1024 * 1024 };
    render(
      <FeedbackDialog
        clip={oversizedClip}
        endpoint=""
        open
        onOpenChange={vi.fn()}
      />,
    );
    expect(screen.getAllByRole("checkbox", { name: /完整视频/ }).at(-1)).toBeDisabled();
    expect(screen.getAllByText(/超过 1 GiB/).length).toBeGreaterThan(0);
  });

  it("uploads with progress reporting and completes", async () => {
    const user = userEvent.setup();
    const unlisten = vi.fn();
    let onProgress: ((progress: FeedbackProgress) => void) | null = null;
    listenMock.mockImplementation(async (handler) => {
      onProgress = handler;
      return unlisten;
    });
    const pending = deferred<FeedbackSubmitResult>();
    submitFeedbackMock.mockReturnValue(pending.promise);
    renderDialog("https://feedback.example.com/api");

    await user.type(screen.getByRole("textbox", { name: /问题描述（必填）/ }), "画面是另一局的内容");
    await user.click(screen.getByRole("button", { name: "上传反馈" }));

    await waitFor(() => expect(submitFeedbackMock).toHaveBeenCalled());
    expect(submitFeedbackMock).toHaveBeenCalledWith(expect.objectContaining({
      clipId: "clip-a",
      category: "mismatch",
      description: "画面是另一局的内容",
      includeFrames: true,
      includeVideo: false,
      endpoint: "https://feedback.example.com/api",
    }));

    act(() => {
      onProgress?.({
        reportId: "vhm-test",
        phase: "building",
        message: "正在打包诊断数据",
        uploadedBytes: 0,
        totalBytes: 0,
      });
      onProgress?.({
        reportId: "vhm-test",
        phase: "uploading",
        message: "正在上传诊断包",
        uploadedBytes: 50,
        totalBytes: 100,
      });
    });
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByText(/正在上传诊断包… 50%/)).toBeVisible();

    pending.resolve(uploadedResult());
    expect(await screen.findByText(/问题反馈已上传/)).toBeVisible();
    expect(screen.getByText(/vhm-test/)).toBeVisible();
    expect(unlisten).toHaveBeenCalled();
  });

  it("falls back to the save dialog and records the destination", async () => {
    const user = userEvent.setup();
    submitFeedbackMock.mockResolvedValue(needsSaveResult());
    saveDialogMock.mockResolvedValue("C:\\reports\\package.zip");
    renderDialog();

    await user.type(screen.getByRole("textbox", { name: /问题描述（必填）/ }), "信息不匹配");
    await user.click(screen.getByRole("button", { name: "生成诊断包" }));

    expect(await screen.findByText(/已保存到 C:\\reports\\package.zip/)).toBeVisible();
    expect(saveFeedbackPackageMock).toHaveBeenCalledWith(
      "C:\\cache\\feedback\\vhm-test\\package.zip",
      "C:\\reports\\package.zip",
    );
    expect(discardFeedbackPackageMock).not.toHaveBeenCalled();
  });

  it("discards the package when the save dialog is cancelled", async () => {
    const user = userEvent.setup();
    submitFeedbackMock.mockResolvedValue(needsSaveResult());
    saveDialogMock.mockResolvedValue(null);
    renderDialog();

    await user.type(screen.getByRole("textbox", { name: /问题描述（必填）/ }), "信息不匹配");
    await user.click(screen.getByRole("button", { name: "生成诊断包" }));

    expect(await screen.findByText(/已取消提交/)).toBeVisible();
    expect(discardFeedbackPackageMock).toHaveBeenCalledWith(
      "C:\\cache\\feedback\\vhm-test\\package.zip",
    );
  });

  it("keeps the submit action disabled until a description is entered", async () => {
    const user = userEvent.setup();
    renderDialog();

    expect(screen.getByRole("button", { name: "生成诊断包" })).toBeDisabled();

    await user.type(
      screen.getByRole("textbox", { name: /问题描述（必填）/ }),
      "信息不匹配",
    );
    expect(screen.getByRole("button", { name: "生成诊断包" })).toBeEnabled();
    expect(submitFeedbackMock).not.toHaveBeenCalled();
  });
});
