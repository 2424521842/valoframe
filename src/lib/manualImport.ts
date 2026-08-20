import type { AccountSummary, ManualClipImportInput } from "../types";

export const NEW_ACCOUNT_OPTION_VALUE = "__new_account__";

export type ManualImportFormState = {
  accountMode: "existing" | "new";
  accountId: string;
  newAccountName: string;
  agentName: string;
  mapName: string;
  gameMode: string;
  note: string;
};

export type ManualImportFormErrors = {
  accountName?: string;
  agentName?: string;
  mapName?: string;
};

export const EMPTY_MANUAL_IMPORT_FORM: ManualImportFormState = {
  accountMode: "existing",
  accountId: "",
  newAccountName: "",
  agentName: "",
  mapName: "",
  gameMode: "",
  note: "",
};

/** Pure form validation; returns the first problem per field. */
export function validateManualImportForm(
  form: ManualImportFormState,
  accounts: readonly AccountSummary[],
): ManualImportFormErrors {
  const errors: ManualImportFormErrors = {};
  if (form.accountMode === "new") {
    if (!form.newAccountName.trim()) {
      errors.accountName = "请输入新账户名称";
    }
  } else if (!accounts.some((account) => account.id === form.accountId)) {
    errors.accountName = "请选择账户";
  }
  if (!form.agentName) {
    errors.agentName = "请选择英雄";
  }
  if (!form.mapName) {
    errors.mapName = "请选择地图";
  }
  return errors;
}

/**
 * Builds the backend import payload. Existing accounts reuse their stable identity key; a
 * `source-<id>` fallback account carries no portable identity, so the backend treats it as a
 * new manual account that keeps its display name.
 */
export function manualImportInputFromForm(
  form: ManualImportFormState,
  accounts: readonly AccountSummary[],
): ManualClipImportInput {
  const existingAccount = form.accountMode === "existing"
    ? accounts.find((account) => account.id === form.accountId) ?? null
    : null;
  const accountName = (existingAccount?.displayName ?? form.newAccountName).trim();
  return {
    accountKey: existingAccount?.id ?? null,
    accountName,
    playerName: null,
    agentName: form.agentName.trim(),
    mapName: form.mapName.trim() || null,
    gameMode: form.gameMode.trim() || null,
    note: form.note.trim() || null,
  };
}

