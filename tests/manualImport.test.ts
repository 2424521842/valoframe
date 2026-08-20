import assert from "node:assert/strict";
import test from "node:test";
import {
  EMPTY_MANUAL_IMPORT_FORM,
  manualImportInputFromForm,
  validateManualImportForm,
} from "../src/lib/manualImport.ts";
import type { ManualImportFormState } from "../src/lib/manualImport.ts";
import type { AccountSummary } from "../src/types";

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
  {
    id: "source-7",
    displayName: "未识别账号",
    sourceName: "",
    clipCount: 1,
    missingCount: 0,
    favoriteCount: 0,
    sizeBytes: 0,
    lastModifiedAt: new Date(0).toISOString(),
    detectedBy: "source-dir",
  },
];

function form(overrides: Partial<ManualImportFormState> = {}): ManualImportFormState {
  return { ...EMPTY_MANUAL_IMPORT_FORM, ...overrides };
}

test("validation requires an account, agent, and map", () => {
  assert.deepEqual(validateManualImportForm(form(), accounts), {
    accountName: "请选择账户",
    agentName: "请选择英雄",
    mapName: "请选择地图",
  });
});

test("validation accepts an existing account selection", () => {
  assert.deepEqual(
    validateManualImportForm(
      form({ accountMode: "existing", accountId: "match-account-1001", agentName: "捷风", mapName: "霓虹町" }),
      accounts,
    ),
    {},
  );
});

test("validation requires a name for a brand-new account", () => {
  assert.deepEqual(
    validateManualImportForm(
      form({ accountMode: "new", agentName: "捷风", mapName: "霓虹町" }),
      accounts,
    ),
    { accountName: "请输入新账户名称" },
  );
});

test("input payload reuses the stable key of an existing account", () => {
  const input = manualImportInputFromForm(
    form({
      accountMode: "existing",
      accountId: "match-account-1001",
      agentName: "捷风",
      mapName: "霓虹町",
      gameMode: "竞技模式",
      note: " 残局反杀 ",
    }),
    accounts,
  );
  assert.deepEqual(input, {
    accountKey: "match-account-1001",
    accountName: "FixtureAlpha#0001",
    playerName: null,
    agentName: "捷风",
    mapName: "霓虹町",
    gameMode: "竞技模式",
    note: "残局反杀",
  });
});

test("input payload treats a new account as a keyless manual account", () => {
  const input = manualImportInputFromForm(
    form({ accountMode: "new", newAccountName: " 小号#1234 ", agentName: "幽影", mapName: "亚海悬城" }),
    accounts,
  );
  assert.equal(input.accountKey, null);
  assert.equal(input.accountName, "小号#1234");
  assert.equal(input.agentName, "幽影");
  assert.equal(input.mapName, "亚海悬城");
});

