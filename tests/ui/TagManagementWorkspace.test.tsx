import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TagManagementWorkspace } from "../../src/screens/TagManagementWorkspace";
import type { Tag } from "../../src/types";

describe("TagManagementWorkspace", () => {
  it("submits the color selected while creating a custom tag", async () => {
    const createdTag: Tag = {
      id: "review",
      label: "复盘",
      color: "teal",
    };
    const onCreateTag = vi.fn(async () => createdTag);

    render(
      <TagManagementWorkspace
        activityMessage=""
        taggedClipCount={0}
        tagUsageCounts={new Map()}
        tags={[]}
        totalClipCount={0}
        onBack={vi.fn()}
        onCreateTag={onCreateTag}
        onDeleteTag={vi.fn(async () => true)}
        onUpdateTag={vi.fn(async () => null)}
        onViewTag={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("标签名称"), {
      target: { value: "复盘" },
    });
    fireEvent.click(screen.getByRole("button", { name: "青绿" }));
    fireEvent.click(screen.getByRole("button", { name: "创建标签" }));

    await waitFor(() => {
      expect(onCreateTag).toHaveBeenCalledWith("复盘", "teal");
    });
  });

  it("renders each saved tag with its persisted color class", () => {
    const tags: Tag[] = [
      { id: "red", label: "赤红标签", color: "red" },
      { id: "teal", label: "青绿标签", color: "teal" },
      { id: "gold", label: "金色标签", color: "gold" },
      { id: "blue", label: "蓝色标签", color: "blue" },
      { id: "green", label: "绿色标签", color: "green" },
    ];

    render(
      <TagManagementWorkspace
        activityMessage=""
        taggedClipCount={0}
        tagUsageCounts={new Map()}
        tags={tags}
        totalClipCount={0}
        onBack={vi.fn()}
        onCreateTag={vi.fn(async () => null)}
        onDeleteTag={vi.fn(async () => true)}
        onUpdateTag={vi.fn(async () => null)}
        onViewTag={vi.fn()}
      />,
    );

    for (const tag of tags) {
      expect(screen.getByText(tag.label)).toHaveClass(`tag--${tag.color}`);
    }
  });

  it("enables save and submits when only an existing tag color changes", async () => {
    const existingTag: Tag = {
      id: "review",
      label: "复盘",
      color: "blue",
    };
    const updatedTag: Tag = {
      ...existingTag,
      color: "green",
    };
    const onUpdateTag = vi.fn(async () => updatedTag);

    render(
      <TagManagementWorkspace
        activityMessage=""
        taggedClipCount={0}
        tagUsageCounts={new Map()}
        tags={[existingTag]}
        totalClipCount={0}
        onBack={vi.fn()}
        onCreateTag={vi.fn(async () => null)}
        onDeleteTag={vi.fn(async () => true)}
        onUpdateTag={onUpdateTag}
        onViewTag={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "编辑" }));
    const saveButton = screen.getByRole("button", { name: "保存" });
    expect(saveButton).toBeDisabled();

    fireEvent.click(screen.getAllByRole("button", { name: "绿色" })[1]);
    expect(saveButton).toBeEnabled();
    fireEvent.click(saveButton);

    await waitFor(() => {
      expect(onUpdateTag).toHaveBeenCalledWith("review", "复盘", "green");
    });
  });
});
