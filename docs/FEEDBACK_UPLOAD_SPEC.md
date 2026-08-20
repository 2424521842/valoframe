# 问题反馈上传接口规范

瓦刻（VALOFRAME）的“问题反馈”功能设计为两种提交方式：

1. **保存文件（当前版本唯一开放的默认方式）**：用户把生成的 valoframe-feedback-*.zip 诊断包保存到本地，再通过 QQ 群、邮件或网盘发送给你。
2. **直传接口（能力已保留，配置入口暂未开放）**：待你的后台就绪后，可在「设置 → 反馈」重新开放接口配置；配置后预览页的“反馈问题”会直接把诊断包上传到该接口，上传失败自动回退为保存文件。

两种方式的诊断包内容完全一致。本文档描述直传接口的接收契约，便于你实现自己的后台。

## 1. 请求

```text
POST {你配置的接口地址}
Content-Type: multipart/form-data; boundary=----ValoframeFeedback<毫秒时间戳>
```

请求体包含两个表单字段：

| 字段 | 类型 | 内容 |
| --- | --- | --- |
| report | text | UTF-8 JSON，反馈元信息（见下） |
| package | file | zip 诊断包，文件名 valoframe-feedback-{clipId}-{unix秒}.zip，MIME application/zip |

### 1.1 report 字段

```json
{
  "schemaVersion": 1,
  "reportId": "vhm-<进程ID>-<毫秒时间戳>",
  "appVersion": "0.2.1",
  "platform": "windows x86_64",
  "clipId": 123,
  "category": "mismatch | playback | metadata | other",
  "description": "用户填写的文字描述（1–2000 字）",
  "contact": "用户选填的 QQ / 邮箱（可为空）",
  "submittedAt": "2026-01-01T12:00:00Z",
  "packageFileName": "valoframe-feedback-123-1767000000.zip",
  "packageBytes": 12345678
}
```

### 1.2 成功响应

任何 2xx 状态码即视为成功，响应体可为空；若返回 JSON，格式不限，客户端会忽略内容。

### 1.3 失败响应

非 2xx 状态码视为失败：客户端会在错误提示中展示状态码与响应体前 200 个字符，并自动回退为“保存诊断包文件”。建议返回简短的 JSON：

```json
{ "error": "upload rejected: package too large" }
```

## 2. 诊断包内容（zip 内条目）

| 条目 | 说明 |
| --- | --- |
| diagnostic.json | 脱敏诊断数据（见下），UTF-8、缩进 JSON |
| frames/frame-01.jpg … | 0–3 张采样帧（用户勾选时，960px 宽 JPEG，按视频 10% / 50% / 90% 时间点抽取；FFmpeg 缺失或失败时按帧跳过并在 package.frameNotes 记录） |
| video/{原文件名} | 用户勾选“附带完整视频”时的视频本体（最大 1 GiB，超出会拒绝勾选） |

zip 使用 Stored（不压缩）方式写入，任何标准 zip 工具都可解压。

## 3. diagnostic.json 结构（v1）

```json
{
  "schemaVersion": 1,
  "generatedAt": "2026-01-01T12:00:00Z",
  "appVersion": "0.2.1",
  "platform": "windows x86_64",
  "clip": {
    "id": 123,
    "clipGroupName": "2025-12-31 21:07",
    "fileName": "xxxx.mp4",
    "extension": "mp4",
    "fileSize": 83886080,
    "modifiedAt": "2025-12-31T13:30:00Z",
    "durationMs": 29480,
    "recordedAt": "2025-12-31T13:07:00Z",
    "coverSource": "missing | file",
    "thumbnailStatus": "ready | failed | ...",
    "fileStatus": "available | missing | trashed",
    "accountDisplayName": "展示用账号名",
    "accountName": "...",
    "playerName": "...",
    "agentName": "捷风",
    "mapName": "源工重镇",
    "gameMode": "标准",
    "metadataStatus": "parsed | ...",
    "matchId": "对局 UUID（可空）",
    "scoreline": "13:10",
    "kda": "21/9/4",
    "agentAvatarUrl": "...",
    "roundLabel": "第 3 回合",
    "weaponName": "狂徒",
    "killCount": 4,
    "matchStartedAt": "2025-12-31T13:02:00Z",
    "combatScore": 312,
    "hasWon": true,
    "officialVideoName": "四杀时刻",
    "officialVideoType": "...",
    "highlightType": 6,
    "metadataSource": "wonderful_db | video_export | highlight_log | leveldb | manual | ...",
    "sourceDirDisplayName": "来源目录名",
    "sourceKind": "aclos | nvidia | tracker | generic",
    "scanMode": "aclos-structured | recursive-mp4",
    "sourceRelativeDir": "对局子目录的相对路径"
  },
  "siblingClips": [
    {
      "id": 124,
      "fileName": "...",
      "fileSize": 0,
      "durationMs": 12000,
      "recordedAt": "...",
      "modifiedAt": "...",
      "officialVideoName": "...",
      "killCount": 1,
      "scoreline": "...",
      "agentName": "...",
      "mapName": "..."
    }
  ],
  "fileCheck": {
    "exists": true,
    "isFile": true,
    "sizeBytes": 83886080,
    "modifiedAt": "2025-12-31T13:30:00Z",
    "indexedSizeBytes": 83886080,
    "indexedModifiedAt": "...",
    "sizeMatches": true,
    "modifiedAtMatches": true
  },
  "package": {
    "framesRequested": true,
    "videoRequested": false,
    "ffmpegAvailable": true,
    "framesCaptured": 3,
    "videoAttached": false,
    "frameNotes": []
  }
}
```

siblingClips 是同一对局（同 clip group 或同 matchId）下最多 20 条其他片段，用于判断“视频与信息不匹配”是分组错误还是文件指派错误。fileCheck 是提交时重新读取的文件系统状态，与索引值不一致（例如 sizeMatches: false）通常意味着文件在扫描后被替换过。

## 4. 隐私与脱敏约定

- 诊断数据**不含**：账号 OpenID / PUUID、matchAccountId、备注、标签、收藏/回收状态、本机绝对路径（只保留文件名与相对目录）、提取文本。
- 采样帧与完整视频属于用户主动勾选的内容；完整视频可能包含游戏语音等个人信息，需按敏感数据处理。
- 客户端只在你配置了接口并主动点击提交后才会发起网络请求；上传仅指向配置的这一个地址，不会向其他域发送数据。

## 5. 接口地址约束（客户端侧）

- 必须为 https://；本机自测允许 http://localhost / http://127.0.0.1 / http://[::1]。
- 长度不超过 300 字符。
- 上传超时 15 分钟，单次诊断包最大约 1 GiB（视频上限 1 GiB + 元数据/采样帧）。
- 上传进度通过应用内 feedback-progress 事件展示（阶段 building / uploading）。

## 6. 一个最小的接收示例（Node.js / Express）

```js
const multer = require("multer");
const upload = multer({ storage: multer.memoryStorage(), limits: { fileSize: 2 * 1024 ** 3 } });

app.post("/api/valoframe-feedback", upload.fields([{ name: "package", maxCount: 1 }]), (req, res) => {
  const report = JSON.parse(req.body.report || "{}");
  const zip = req.files?.package?.[0];
  console.log("report", report.reportId, report.category, report.clipId);
  // 把 zip.buffer 存入磁盘或对象存储，report 写入数据库
  res.json({ ok: true });
});
```
