import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ManualClipImportDialog } from "../../src/components/ManualClipImportDialog";
import type { AccountSummary, ManualClipImportInput, PendingManualClip } from "../../src/types";

const accounts: AccountSummary[] = [
  {
    id: "match-account-1001",
    displayName: "FixtureAlpha#0001",
    sourceName: "",
    clipCount: 3,
    missingCount: 0,
    favoriteCount: 1,
    sizeBytes: 0,
    lastModifiedAt: new Date(0).toISOString(),
    detectedBy: "metadata",
  },
];

const clip: PendingManualClip = {
  id: "pending-1",
  sourceDirId: "source-nvidia",
  sourceDirName: "NVIDIA 录屏",
  filePath: "D:\\Videos\\NVIDIA\\Valorant clip.mp4",
  fileName: "Valorant clip.mp4",
  fileSize: 84_500_000,
  modifiedAt: "2026-07-02T22:06:00Z",
  sourceRelativeDir: "",
  ignored: false,
  firstDiscoveredAt: "2026-07-02T22:45:00Z",
};

function renderDialog(
  onSubmit: (input: ManualClipImportInput) => void = vi.fn(),
  state: { isSubmitting?: boolean; error?: string | null } = {},
) {
  render(
    <ManualClipImportDialog
      open={true}
      clip={clip}
      accounts={accounts}
      agentNames={["捷风", "幽影"]}
      mapNames={["霓虹町", "亚海悬城"]}
      gameModes={["竞技模式"]}
      isSubmitting={state.isSubmitting ?? false}
      error={state.error ?? null}
      onOpenChange={vi.fn()}
      onSubmit={onSubmit}
    />,
  );
  return onSubmit;
}

describe("ManualClipImportDialog", () => {
  it("shows the pending file and blocks empty submissions with field errors", async () => {
    const user = userEvent.setup();
    const onSubmit = renderDialog();

    expect(screen.getByRole("dialog", { name: "录入 NVIDIA 视频" })).toBeVisible();
    expect(screen.getByText("Valorant clip.mp4")).toBeVisible();
    expect(screen.getByText(/NVIDIA 录屏没有对局元数据/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "录入到素材库" }));
    expect(await screen.findByText("请选择账户")).toBeVisible();
    expect(screen.getByText("请选择英雄")).toBeVisible();
    expect(screen.getByText("请选择地图")).toBeVisible();
    expect(screen.getByRole("combobox", { name: "选择账户" })).toHaveFocus();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps actions outside the keyboard-scrollable preview and form region", () => {
    renderDialog();

    const scrollRegion = screen.getByLabelText("视频预览与分类信息");
    const submitButton = screen.getByRole("button", { name: "录入到素材库" });
    expect(scrollRegion).toHaveAttribute("tabindex", "0");
    expect(scrollRegion).toContainElement(screen.getByLabelText("待录入文件"));
    expect(scrollRegion).not.toContainElement(submitButton);
    expect(submitButton.closest(".manual-import-actions")).not.toBeNull();
  });

  it("submits a new-account payload after filling the classification", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderDialog(onSubmit);

    await user.click(screen.getByRole("combobox", { name: "选择账户" }));
    await user.click(await screen.findByRole("option", { name: /新添加账户/ }));
    const accountInput = screen.getByRole("textbox", { name: /新账户名称/ });
    await user.type(accountInput, "小号#1234");

    await user.click(screen.getByRole("combobox", { name: "选择英雄" }));
    await user.click(await screen.findByRole("option", { name: "捷风" }));

    await user.click(screen.getByRole("combobox", { name: "选择地图" }));
    await user.click(await screen.findByRole("option", { name: "霓虹町" }));

    await user.click(screen.getByRole("combobox", { name: "选择模式" }));
    await user.click(await screen.findByRole("option", { name: "竞技模式" }));

    const note = screen.getByRole("textbox", { name: /备注/ });
    await user.type(note, "残局反杀");

    await user.click(screen.getByRole("button", { name: "录入到素材库" }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith({
      accountKey: null,
      accountName: "小号#1234",
      playerName: null,
      agentName: "捷风",
      mapName: "霓虹町",
      gameMode: "竞技模式",
      note: "残局反杀",
    });
  });

  it("reuses an existing account identity when one is selected", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderDialog(onSubmit);

    await user.click(screen.getByRole("combobox", { name: "选择账户" }));
    await user.click(await screen.findByRole("option", { name: "FixtureAlpha#0001" }));
    await user.click(screen.getByRole("combobox", { name: "选择英雄" }));
    await user.click(await screen.findByRole("option", { name: "幽影" }));
    await user.click(screen.getByRole("combobox", { name: "选择地图" }));
    await user.click(await screen.findByRole("option", { name: "亚海悬城" }));

    await user.click(screen.getByRole("button", { name: "录入到素材库" }));
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      accountKey: "match-account-1001",
      accountName: "FixtureAlpha#0001",
    }));
  });

  it("renders select options in a portal outside the clipped scroll region", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.click(screen.getByRole("combobox", { name: "选择英雄" }));
    const listbox = await screen.findByRole("listbox");
    expect(listbox.closest(".manual-import-scroll")).toBeNull();
    expect(screen.getByRole("option", { name: "捷风" })).toBeVisible();
  });

  it("streams the pending recording so the user can watch it before classifying", () => {
    renderDialog();

    const file = screen.getByLabelText("待录入文件");
    const video = file.querySelector("video");
    expect(video).not.toBeNull();
    // Keyed by pending id so switching rows reloads rather than reusing the previous buffer.
    expect(video).toHaveAttribute("src", expect.stringContaining("pending/pending-1"));
    expect(video).toHaveAttribute("controls");
    expect(screen.getByText(/85 MB/)).toBeVisible();
  });

  it("keeps the form usable when the recording cannot be decoded for preview", () => {
    renderDialog();

    const video = screen.getByLabelText("待录入文件").querySelector("video");
    fireEvent.error(video!);

    expect(screen.getByText(/无法在应用内预览该视频/)).toBeVisible();
    expect(screen.getByRole("combobox", { name: "选择英雄" })).toBeVisible();
    expect(screen.getByRole("button", { name: "录入到素材库" })).toBeEnabled();
  });

  it("keeps submission feedback readable without allowing a duplicate submit", () => {
    renderDialog(vi.fn(), { isSubmitting: true, error: "录入失败，请重试" });

    expect(screen.getByRole("alert")).toHaveTextContent("录入失败，请重试");
    expect(screen.getByRole("button", { name: "正在录入…" })).toBeDisabled();
  });
});
