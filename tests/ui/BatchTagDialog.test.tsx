import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { BatchTagDialog } from "../../src/components/BatchTagDialog";
import { mockClips } from "../../src/data/mockData";
import type { Tag } from "../../src/types";

const tag: Tag = { id: "clutch", label: "残局", color: "red" };

describe("BatchTagDialog request context", () => {
  it("does not write feedback from an old request into a new selection context", async () => {
    const user = userEvent.setup();
    const update = deferred<boolean>();
    const onSetTag = vi.fn(() => update.promise);
    const view = render(
      <BatchTagDialog
        isBusy={false}
        open
        selectedClips={[mockClips[0]]}
        tags={[tag]}
        onCreateTag={vi.fn(async () => null)}
        onOpenChange={vi.fn()}
        onSetTag={onSetTag}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "移除残局标签" }));
    expect(onSetTag).toHaveBeenCalledTimes(1);

    view.rerender(
      <BatchTagDialog
        isBusy={false}
        open
        selectedClips={[mockClips[1]]}
        tags={[tag]}
        onCreateTag={vi.fn(async () => null)}
        onOpenChange={vi.fn()}
        onSetTag={onSetTag}
      />,
    );
    await act(async () => update.resolve(true));

    expect(screen.queryByText("已更新标签：残局")).not.toBeInTheDocument();
    expect(screen.getByText("已选择 1 条素材；勾选会应用到全部素材，取消会从全部素材移除。")).toBeVisible();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve;
  });
  return { promise, resolve };
}
