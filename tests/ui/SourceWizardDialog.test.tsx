import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SourceWizardDialog } from "../../src/components/SourceWizardDialog";
import type { RegisterScanSourceResult } from "../../src/types";

function registeredResult(overrides: Partial<RegisterScanSourceResult> = {}): RegisterScanSourceResult {
  return {
    sources: [],
    createdCount: 1,
    duplicateCount: 0,
    normalizedRootPath: "D:\\Tracker\\Clips",
    requiresOverlapConfirmation: false,
    overlaps: [],
    ...overrides,
  };
}

describe("SourceWizardDialog", () => {
  it("honors the requested initial source and explains the NVIDIA privacy boundary", async () => {
    const user = userEvent.setup();
    render(
      <SourceWizardDialog
        initialSourceKind="generic"
        open
        onChooseDirectory={vi.fn(async () => null)}
        onOpenChange={vi.fn()}
        onRegister={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: /其他录制目录/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText(/NVIDIA 私有元数据/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /NVIDIA 录屏/ }));

    expect(screen.getByRole("button", { name: /NVIDIA 录屏/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/NVIDIA App 保存录屏的 MP4 目录/)).toBeVisible();
    expect(screen.getByText(/NVIDIA 私有元数据/)).toBeVisible();
  });

  it("keeps import guidance available while source interactions are temporarily blocked", () => {
    render(
      <SourceWizardDialog
        initialSourceKind="nvidia"
        interactionDisabledReason="当前扫描任务正在运行，请先取消扫描。"
        open
        onChooseDirectory={vi.fn(async () => null)}
        onOpenChange={vi.fn()}
        onRegister={vi.fn()}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("请先取消扫描");
    expect(screen.getByText(/NVIDIA 私有元数据/)).toBeVisible();
    expect(screen.getByRole("button", { name: "选择目录" })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "来源显示名称" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "添加并首次同步" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "取消" })).toBeEnabled();
  });

  it("collects source type, directory, display name, and startup policy", async () => {
    const user = userEvent.setup();
    const onChooseDirectory = vi.fn(async () => "D:\\Tracker\\Clips");
    const onRegister = vi.fn(async () => registeredResult());
    const onOpenChange = vi.fn();
    render(
      <SourceWizardDialog
        open
        onChooseDirectory={onChooseDirectory}
        onOpenChange={onOpenChange}
        onRegister={onRegister}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Tracker 录制/ }));
    await user.click(screen.getByRole("button", { name: "选择目录" }));
    await user.clear(screen.getByRole("textbox", { name: "来源显示名称" }));
    await user.type(screen.getByRole("textbox", { name: "来源显示名称" }), "排位录像");
    await user.click(screen.getByRole("button", { name: "添加并首次同步" }));

    expect(onChooseDirectory).toHaveBeenCalledWith("tracker");
    expect(onRegister).toHaveBeenCalledWith({
      sourceKind: "tracker",
      scanRootPath: "D:\\Tracker\\Clips",
      displayName: "排位录像",
      enabled: true,
      allowOverlap: false,
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("requires an explicit second action before accepting an overlapping root", async () => {
    const user = userEvent.setup();
    const onRegister = vi.fn()
      .mockResolvedValueOnce(registeredResult({
        sources: [],
        createdCount: 0,
        requiresOverlapConfirmation: true,
        overlaps: [{
          id: "9",
          displayName: "已有来源",
          sourceKind: "generic",
          scanRootPath: "D:\\Tracker",
        }],
      }))
      .mockResolvedValueOnce(registeredResult());
    const onOpenChange = vi.fn();
    render(
      <SourceWizardDialog
        open
        onChooseDirectory={vi.fn(async () => "D:\\Tracker\\Clips")}
        onOpenChange={onOpenChange}
        onRegister={onRegister}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择目录" }));
    await user.click(screen.getByRole("button", { name: "添加并首次同步" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已有来源");
    expect(onOpenChange).not.toHaveBeenCalledWith(false);

    await user.click(screen.getByRole("button", { name: "确认重叠并继续" }));
    expect(onRegister).toHaveBeenLastCalledWith(expect.objectContaining({ allowOverlap: true }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
