#!/usr/bin/env python3
"""Upload one verified VALOFRAME installer to a fixed Lanzou Cloud folder.

Lanzou Cloud does not publish a supported upload API. This client intentionally
uses only the small-file path exposed by the web application, keeps TLS
verification enabled, and refuses to bypass the service's file-size or file-type
limits. The login cookie is read from LANZOU_COOKIE so it never appears in the
command line.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Protocol


DISK_URL = "https://pc.woozooo.com/mydisk.php"
MANAGE_URL = "https://pc.woozooo.com/doupload.php"
UPLOAD_URL = "https://pc.woozooo.com/html5up.php"
MAX_FILE_BYTES = 100 * 1024 * 1024
USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36"
)
INSTALLER_PATTERN = re.compile(
    r"^VALOFRAME-(?P<version>(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))-x64-setup\.exe$"
)
TAG_PATTERN = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
COOKIE_NAME_PATTERN = re.compile(r"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$")


class LanzouUploadError(RuntimeError):
    """A safe, user-facing synchronization failure."""


@dataclass(frozen=True)
class HttpResponse:
    status: int
    body: bytes


class Transport(Protocol):
    def request(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str],
        data: bytes | None = None,
    ) -> HttpResponse: ...


class UrlLibTransport:
    def __init__(self, timeout: float = 30.0) -> None:
        self.timeout = timeout

    def request(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str],
        data: bytes | None = None,
    ) -> HttpResponse:
        request = urllib.request.Request(
            url=url,
            data=data,
            headers=dict(headers),
            method=method,
        )
        try:
            # urllib's default HTTPS context verifies both certificate and hostname.
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                return HttpResponse(status=response.status, body=response.read())
        except urllib.error.HTTPError as error:
            error.read()
            raise LanzouUploadError(
                f"Lanzou Cloud returned HTTP {error.code} for {urllib.parse.urlsplit(url).path}."
            ) from None
        except urllib.error.URLError as error:
            raise LanzouUploadError(f"Could not reach Lanzou Cloud: {error.reason}") from None


def parse_cookie(raw_cookie: str) -> str:
    if not raw_cookie or not raw_cookie.strip():
        raise LanzouUploadError("LANZOU_COOKIE is empty.")
    if "\r" in raw_cookie or "\n" in raw_cookie:
        raise LanzouUploadError("LANZOU_COOKIE must be a single HTTP Cookie header line.")

    cookies: dict[str, str] = {}
    for segment in raw_cookie.split(";"):
        segment = segment.strip()
        if not segment:
            continue
        if "=" not in segment:
            raise LanzouUploadError("LANZOU_COOKIE contains a malformed segment.")
        name, value = segment.split("=", 1)
        name = name.strip()
        value = value.strip()
        if not COOKIE_NAME_PATTERN.fullmatch(name) or not value:
            raise LanzouUploadError("LANZOU_COOKIE contains an invalid cookie name or value.")
        cookies[name] = value

    missing = sorted({"ylogin", "phpdisk_info"} - cookies.keys())
    if missing:
        raise LanzouUploadError(
            "LANZOU_COOKIE is missing required login cookie(s): " + ", ".join(missing)
        )
    return "; ".join(f"{name}={value}" for name, value in cookies.items())


def read_expected_sha256(checksums_path: Path, filename: str) -> str:
    matches: list[str] = []
    try:
        lines = checksums_path.read_text(encoding="utf-8-sig").splitlines()
    except OSError as error:
        raise LanzouUploadError(f"Could not read checksum manifest: {error}") from None

    for line in lines:
        match = re.fullmatch(r"([0-9A-Fa-f]{64}) [ *](.+)", line)
        if match and match.group(2) == filename:
            matches.append(match.group(1).lower())
    if len(matches) != 1:
        raise LanzouUploadError(
            f"SHA256SUMS.txt must contain exactly one checksum for {filename}; found {len(matches)}."
        )
    return matches[0]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise LanzouUploadError(f"Could not hash installer: {error}") from None
    return digest.hexdigest()


def validate_inputs(file_path: Path, tag: str, repository: str, folder_id: int) -> str:
    if not file_path.is_file():
        raise LanzouUploadError(f"Installer does not exist: {file_path}")
    installer_match = INSTALLER_PATTERN.fullmatch(file_path.name)
    if installer_match is None:
        raise LanzouUploadError("Installer name must be VALOFRAME-VERSION-x64-setup.exe.")
    if TAG_PATTERN.fullmatch(tag) is None:
        raise LanzouUploadError("Release tag must use canonical vMAJOR.MINOR.PATCH syntax.")
    version = installer_match.group("version")
    if tag != f"v{version}":
        raise LanzouUploadError("Installer version and release tag do not match.")
    if REPOSITORY_PATTERN.fullmatch(repository) is None:
        raise LanzouUploadError("Repository must use OWNER/REPOSITORY syntax.")
    if folder_id <= 0:
        raise LanzouUploadError("LANZOU_FOLDER_ID must identify a shared non-root folder.")
    file_size = file_path.stat().st_size
    if file_size <= 0 or file_size > MAX_FILE_BYTES:
        raise LanzouUploadError("Installer must be between 1 byte and the official 100 MiB limit.")
    return str(file_path.resolve())


def _json_object(response: HttpResponse, operation: str) -> dict[str, Any]:
    if response.status < 200 or response.status >= 300:
        raise LanzouUploadError(f"Lanzou Cloud {operation} returned HTTP {response.status}.")
    try:
        payload = json.loads(response.body.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise LanzouUploadError(f"Lanzou Cloud {operation} returned invalid JSON.") from None
    if not isinstance(payload, dict):
        raise LanzouUploadError(f"Lanzou Cloud {operation} returned an unexpected payload.")
    return payload


def _multipart_body(fields: Mapping[str, str], file_path: Path) -> tuple[bytes, str]:
    boundary = "----VALOFRAME" + secrets.token_hex(20)
    chunks: list[bytes] = []
    for name, value in fields.items():
        chunks.extend(
            [
                f"--{boundary}\r\n".encode("ascii"),
                f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode("ascii"),
                value.encode("utf-8"),
                b"\r\n",
            ]
        )
    try:
        file_bytes = file_path.read_bytes()
    except OSError as error:
        raise LanzouUploadError(f"Could not read installer for upload: {error}") from None
    chunks.extend(
        [
            f"--{boundary}\r\n".encode("ascii"),
            (
                f'Content-Disposition: form-data; name="upload_file"; '
                f'filename="{file_path.name}"\r\n'
            ).encode("utf-8"),
            b"Content-Type: application/octet-stream\r\n\r\n",
            file_bytes,
            b"\r\n",
            f"--{boundary}--\r\n".encode("ascii"),
        ]
    )
    return b"".join(chunks), f"multipart/form-data; boundary={boundary}"


def _valid_share_url(value: str) -> bool:
    try:
        parsed = urllib.parse.urlsplit(value)
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


class LanzouClient:
    def __init__(self, cookie_header: str, transport: Transport) -> None:
        self.transport = transport
        self.headers = {
            "Accept-Language": "zh-CN,zh;q=0.9",
            "Cookie": cookie_header,
            "Referer": DISK_URL,
            "User-Agent": USER_AGENT,
        }

    def authenticate(self) -> None:
        response = self.transport.request("GET", DISK_URL, self.headers)
        if response.status != 200:
            raise LanzouUploadError(f"Lanzou Cloud login check returned HTTP {response.status}.")
        html = response.body.decode("utf-8", errors="replace")
        if "网盘用户登录" in html or "name=\"username\"" in html:
            raise LanzouUploadError("LANZOU_COOKIE is expired or invalid.")

    def _post_form(self, fields: Mapping[str, str], operation: str) -> dict[str, Any]:
        body = urllib.parse.urlencode(fields).encode("ascii")
        headers = dict(self.headers)
        headers["Content-Type"] = "application/x-www-form-urlencoded"
        response = self.transport.request("POST", MANAGE_URL, headers, body)
        return _json_object(response, operation)

    def folder_share(self, folder_id: int) -> tuple[str, str]:
        payload = self._post_form(
            {"task": "18", "folder_id": str(folder_id)}, "folder-share lookup"
        )
        info = payload.get("info")
        if not isinstance(info, dict) or not info.get("name"):
            raise LanzouUploadError("LANZOU_FOLDER_ID is invalid or is not accessible to this account.")
        url = str(info.get("new_url", ""))
        if not _valid_share_url(url):
            raise LanzouUploadError("Lanzou Cloud returned an invalid or non-HTTPS folder share URL.")
        password = str(info.get("pwd", "")) if str(info.get("onof", "0")) == "1" else ""
        if password and re.fullmatch(r"[A-Za-z0-9]{1,12}", password) is None:
            raise LanzouUploadError("Lanzou Cloud returned an unsafe folder password value.")
        return url, password

    def list_files(self, folder_id: int) -> list[dict[str, Any]]:
        files: list[dict[str, Any]] = []
        for page in range(1, 101):
            payload = self._post_form(
                {"task": "5", "folder_id": str(folder_id), "pg": str(page)},
                "folder listing",
            )
            if payload.get("info") == 0:
                return files
            page_files = payload.get("text")
            if not isinstance(page_files, list):
                raise LanzouUploadError("Lanzou Cloud returned an invalid folder listing.")
            for item in page_files:
                if isinstance(item, dict) and "id" in item and "name_all" in item:
                    files.append(item)
        raise LanzouUploadError("Lanzou Cloud folder listing exceeded the 100-page safety limit.")

    def file_description(self, file_id: int) -> str:
        payload = self._post_form(
            {"task": "12", "file_id": str(file_id)}, "file-description lookup"
        )
        description = payload.get("info")
        if not isinstance(description, str):
            raise LanzouUploadError("Lanzou Cloud returned an invalid file description.")
        return description

    def upload(self, file_path: Path, folder_id: int) -> int:
        body, content_type = _multipart_body(
            {
                "task": "1",
                "vie": "2",
                "ve": "2",
                "id": "WU_FILE_0",
                "folder_id_bb_n": str(folder_id),
                "name": file_path.name,
            },
            file_path,
        )
        headers = dict(self.headers)
        headers["Content-Type"] = content_type
        response = self.transport.request("POST", UPLOAD_URL, headers, body)
        payload = _json_object(response, "file upload")
        uploaded = payload.get("text")
        if payload.get("zt") != 1 or not isinstance(uploaded, list) or not uploaded:
            raise LanzouUploadError("Lanzou Cloud rejected the installer upload.")
        try:
            file_id = int(uploaded[0]["id"])
        except (KeyError, TypeError, ValueError):
            raise LanzouUploadError("Lanzou Cloud upload response did not include a file ID.") from None
        if file_id <= 0:
            raise LanzouUploadError("Lanzou Cloud returned an invalid uploaded file ID.")
        return file_id

    def set_file_description(self, file_id: int, description: str) -> bool:
        payload = self._post_form(
            {"task": "11", "file_id": str(file_id), "desc": description},
            "file-description update",
        )
        return payload.get("zt") == 1

    def move_to_recycle_bin(self, file_id: int) -> bool:
        payload = self._post_form(
            {"task": "6", "file_id": str(file_id)}, "upload rollback"
        )
        return payload.get("zt") == 1


def synchronize(
    *,
    file_path: Path,
    checksums_path: Path,
    tag: str,
    repository: str,
    folder_id: int,
    cookie: str,
    transport: Transport,
) -> dict[str, Any]:
    resolved_file = validate_inputs(file_path, tag, repository, folder_id)
    expected_sha256 = read_expected_sha256(checksums_path, file_path.name)
    actual_sha256 = sha256_file(file_path)
    if actual_sha256 != expected_sha256:
        raise LanzouUploadError("Installer does not match its SHA256SUMS.txt entry.")

    client = LanzouClient(parse_cookie(cookie), transport)
    client.authenticate()
    folder_url, folder_password = client.folder_share(folder_id)
    description = f"VALOFRAME {tag} SHA-256: {actual_sha256}"

    matching = [item for item in client.list_files(folder_id) if item.get("name_all") == file_path.name]
    if len(matching) > 1:
        raise LanzouUploadError("More than one same-named installer already exists in the target folder.")
    if matching:
        try:
            existing_id = int(matching[0]["id"])
        except (TypeError, ValueError):
            raise LanzouUploadError("Existing Lanzou Cloud file has an invalid ID.") from None
        if client.file_description(existing_id) != description:
            raise LanzouUploadError(
                "A same-named installer already exists without the expected SHA-256 marker; "
                "refusing to overwrite it."
            )
        file_id = existing_id
        status = "already-present"
    else:
        file_id = client.upload(file_path, folder_id)
        if not client.set_file_description(file_id, description):
            rolled_back = client.move_to_recycle_bin(file_id)
            suffix = " The new file was moved to the recycle bin." if rolled_back else ""
            raise LanzouUploadError("Could not attach the SHA-256 marker to the uploaded file." + suffix)
        status = "uploaded"

    return {
        "schemaVersion": 1,
        "status": status,
        "tag": tag,
        "repository": repository,
        "sourceFile": resolved_file,
        "file": {
            "id": file_id,
            "name": file_path.name,
            "size": file_path.stat().st_size,
            "sha256": actual_sha256,
        },
        "folder": {
            "id": folder_id,
            "url": folder_url,
            "password": folder_password,
        },
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", required=True, type=Path)
    parser.add_argument("--checksums", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--folder-id", required=True, type=int)
    parser.add_argument("--result", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=30.0)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        result = synchronize(
            file_path=args.file,
            checksums_path=args.checksums,
            tag=args.tag,
            repository=args.repository,
            folder_id=args.folder_id,
            cookie=os.environ.get("LANZOU_COOKIE", ""),
            transport=UrlLibTransport(timeout=args.timeout),
        )
        args.result.parent.mkdir(parents=True, exist_ok=True)
        args.result.write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        password_state = "configured" if result["folder"]["password"] else "not configured"
        print(
            f"Lanzou sync {result['status']}: {result['file']['name']} "
            f"({result['file']['sha256']}); folder password {password_state}."
        )
        return 0
    except LanzouUploadError as error:
        print(f"Lanzou sync failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
