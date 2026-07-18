import assert from "node:assert/strict";
import test from "node:test";
import { deriveActiveFilters } from "../src/lib/activeFilters.ts";

test("deriveActiveFilters returns only active conditions in stable order", () => {
  assert.deepEqual(
    deriveActiveFilters({
      libraryMode: "favorites",
      query: " 残局 ",
      accountId: "account-1",
      accountLabel: "FixtureBravo#0002",
      sourceDirId: "all",
      sourceDirLabel: "",
      agentName: "贤者",
      mapName: "all",
      gameMode: "all",
      tagId: "tag-1",
      tagLabel: "三杀",
      fileStatus: "missing",
    }),
    [
      { key: "mode", label: "收藏" },
      { key: "query", label: "搜索：残局" },
      { key: "account", label: "账号：FixtureBravo#0002" },
      { key: "agent", label: "英雄：贤者" },
      { key: "tag", label: "标签：三杀" },
      { key: "file-status", label: "状态：文件丢失" },
    ],
  );
});

test("deriveActiveFilters omits defaults", () => {
  assert.deepEqual(
    deriveActiveFilters({
      libraryMode: "all",
      query: "",
      accountId: "all",
      accountLabel: "",
      sourceDirId: "all",
      sourceDirLabel: "",
      agentName: "all",
      mapName: "all",
      gameMode: "all",
      tagId: "all",
      tagLabel: "",
      fileStatus: "all",
    }),
    [],
  );
});

test("recycle bin is a distinct library mode instead of the missing-file filter", () => {
  assert.deepEqual(
    deriveActiveFilters({
      libraryMode: "trash",
      query: "",
      accountId: "all",
      accountLabel: "",
      sourceDirId: "all",
      sourceDirLabel: "",
      agentName: "all",
      mapName: "all",
      gameMode: "all",
      tagId: "all",
      tagLabel: "",
      fileStatus: "all",
    }),
    [{ key: "mode", label: "回收站" }],
  );
});
