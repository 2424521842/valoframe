import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");
const workflow = read(".github/workflows/lanzou-sync.yml");
const uploader = read("scripts/release/upload_lanzou.py");
const readme = read("README.md");

test("Lanzou sync follows only a successful stable release or an explicit retry", () => {
  assert.match(
    workflow,
    /workflow_run:\r?\n\s+workflows:\r?\n\s+- Stable signed release[\s\S]*?types:\r?\n\s+- completed/u,
  );
  assert.match(workflow, /workflow_dispatch:[\s\S]*?tag:/u);
  assert.match(workflow, /workflow_run\.conclusion == 'success'/u);
  assert.match(workflow, /workflow_run\.event == 'push'/u);
  assert.match(workflow, /EXPECTED_REPOSITORY: 2424521842\/valoframe/u);
  assert.match(workflow, /The requested release is not a published stable release/u);
  assert.match(workflow, /completed stable workflow SHA does not match its tag/u);
});

test("Lanzou sync executes only trusted default-branch code", () => {
  assert.match(
    workflow,
    /actions\/checkout@d23441a48e516b6c34aea4fa41551a30e30af803/u,
  );
  const checkoutStep = workflow
    .split(/\r?\n(?= {6}- name:)/u)
    .find((step) => step.includes("actions/checkout@"));
  assert.ok(checkoutStep);
  assert.doesNotMatch(checkoutStep, /\n\s+ref:/u);
  assert.match(checkoutStep, /persist-credentials: false/u);
  assert.doesNotMatch(workflow, /pip install|curl\s|wget\s|uses: [^\s]+@(?![0-9a-f]{40})/u);
});

test("Lanzou credentials stay in step-scoped configuration", () => {
  assert.match(workflow, /LANZOU_COOKIE: \$\{\{ secrets\.LANZOU_COOKIE \}\}/u);
  assert.match(workflow, /LANZOU_FOLDER_ID: \$\{\{ vars\.LANZOU_FOLDER_ID \}\}/u);
  const jobEnvironment = workflow.slice(
    workflow.indexOf("    env:"),
    workflow.indexOf("    steps:"),
  );
  assert.doesNotMatch(jobEnvironment, /LANZOU_COOKIE|LANZOU_FOLDER_ID|GH_TOKEN/u);
  assert.doesNotMatch(workflow, /--cookie/u);
  assert.match(uploader, /os\.environ\.get\("LANZOU_COOKIE", ""\)/u);
  assert.doesNotMatch(uploader, /verify\s*=\s*False|ignore_limits|set_max_size/u);
});

test("Lanzou sync consumes only the checksum-bound public installer", () => {
  const downloadIndex = workflow.indexOf(
    "name: Download the exact public installer and checksum manifest",
  );
  const uploadIndex = workflow.indexOf(
    "name: Upload the checksum-bound installer to the fixed folder",
  );
  const notesIndex = workflow.indexOf(
    "name: Publish the verified mirror details in the release notes",
  );
  assert.ok(downloadIndex > 0 && uploadIndex > downloadIndex && notesIndex > uploadIndex);
  assert.match(workflow, /gh release download "\$RELEASE_TAG"/u);
  assert.match(workflow, /--pattern "\$INSTALLER_NAME"/u);
  assert.match(workflow, /--pattern SHA256SUMS\.txt/u);
  assert.match(workflow, /upload_lanzou\.py[\s\S]*?--checksums/u);
  assert.match(workflow, /update_lanzou_release_notes\.py/u);
  assert.match(workflow, /gh release edit "\$RELEASE_TAG"/u);
});

test("repository home page keeps the stable Lanzou folder discoverable", () => {
  assert.match(readme, /https:\/\/wwbfc\.lanzoue\.com\/b01euqdb0h/u);
  assert.match(readme, /提取码：`h0wz`/u);
  assert.match(readme, /以后稳定版也会自动同步到这个固定文件夹/u);
  assert.doesNotMatch(readme, /镜像上传后/u);
});
