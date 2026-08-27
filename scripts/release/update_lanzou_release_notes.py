#!/usr/bin/env python3
"""Insert or replace the verified Lanzou mirror block in GitHub release notes."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit


START_MARKER = "<!-- valoframe-lanzou-mirror:start -->"
END_MARKER = "<!-- valoframe-lanzou-mirror:end -->"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
INSTALLER_PATTERN = re.compile(
    r"^VALOFRAME-(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)-x64-setup\.exe$"
)


class NotesError(RuntimeError):
    pass


def _valid_url(value: str) -> bool:
    try:
        parsed = urlsplit(value)
    except ValueError:
        return False
    hostname = (parsed.hostname or "").lower()
    return (
        parsed.scheme == "https"
        and parsed.username is None
        and parsed.password is None
        and re.fullmatch(r"(?:[a-z0-9-]+\.)*lanzou[a-z]\.com", hostname) is not None
        and bool(parsed.path)
    )


def load_result(path: Path) -> dict[str, Any]:
    try:
        result = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise NotesError(f"Could not read Lanzou sync result: {error}") from None
    if not isinstance(result, dict) or result.get("schemaVersion") != 1:
        raise NotesError("Lanzou sync result has an unsupported schema.")
    if result.get("status") not in {"uploaded", "already-present"}:
        raise NotesError("Lanzou sync did not report a publishable status.")
    return result


def build_block(result: dict[str, Any]) -> str:
    file_info = result.get("file")
    folder_info = result.get("folder")
    if not isinstance(file_info, dict) or not isinstance(folder_info, dict):
        raise NotesError("Lanzou sync result is missing file or folder details.")

    name = str(file_info.get("name", ""))
    sha256 = str(file_info.get("sha256", ""))
    url = str(folder_info.get("url", ""))
    password = str(folder_info.get("password", ""))
    if INSTALLER_PATTERN.fullmatch(name) is None:
        raise NotesError("Lanzou sync result contains an invalid installer name.")
    if SHA256_PATTERN.fullmatch(sha256) is None:
        raise NotesError("Lanzou sync result contains an invalid SHA-256.")
    if not _valid_url(url):
        raise NotesError("Lanzou sync result contains an invalid folder URL.")
    if password and re.fullmatch(r"[A-Za-z0-9]{1,12}", password) is None:
        raise NotesError("Lanzou sync result contains an unsafe folder password.")

    password_text = f"`{password}`" if password else "无"
    return "\n".join(
        [
            START_MARKER,
            "### 蓝奏云（备用）",
            "",
            f"[**打开蓝奏云下载文件夹**]({url})",
            "",
            f"- 提取码：{password_text}",
            f"- 文件：`{name}`",
            f"- SHA-256：`{sha256}`",
            "",
            "> 该镜像在 GitHub 稳定版公开并完成校验后自动同步；文件名与 SHA-256 应与上方 GitHub 安装包一致。",
            END_MARKER,
        ]
    )


def update_notes(notes: str, block: str) -> str:
    normalized = notes.replace("\r\n", "\n").rstrip() + "\n"
    marker_pattern = re.compile(
        re.escape(START_MARKER) + r".*?" + re.escape(END_MARKER), re.DOTALL
    )
    if marker_pattern.search(normalized):
        return marker_pattern.sub(block, normalized, count=1).rstrip() + "\n"

    download_heading = re.search(r"(?m)^## 下载\s*$", normalized)
    if download_heading:
        remainder = normalized[download_heading.end() :]
        next_heading = re.search(r"(?m)^## (?!下载\s*$).+$", remainder)
        if next_heading:
            insert_at = download_heading.end() + next_heading.start()
            before = normalized[:insert_at].rstrip()
            after = normalized[insert_at:].lstrip("\n")
            return f"{before}\n\n{block}\n\n{after}".rstrip() + "\n"

    return f"{normalized.rstrip()}\n\n## 下载镜像\n\n{block}\n"


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--notes", required=True, type=Path)
    parser.add_argument("--result", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        notes = args.notes.read_text(encoding="utf-8")
        block = build_block(load_result(args.result))
        updated = update_notes(notes, block)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(updated, encoding="utf-8")
        return 0
    except (OSError, NotesError) as error:
        print(f"Could not update release notes: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
