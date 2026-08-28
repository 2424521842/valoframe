from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
import urllib.parse
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import update_lanzou_release_notes as notes_module  # noqa: E402
import upload_lanzou as upload_module  # noqa: E402


class FakeTransport:
    def __init__(self, responses: list[upload_module.HttpResponse]) -> None:
        self.responses = list(responses)
        self.requests: list[tuple[str, str, dict[str, str], bytes | None]] = []

    def request(self, method: str, url: str, headers, data=None):
        self.requests.append((method, url, dict(headers), data))
        if not self.responses:
            raise AssertionError(f"Unexpected request: {method} {url}")
        return self.responses.pop(0)


def json_response(payload) -> upload_module.HttpResponse:
    return upload_module.HttpResponse(200, json.dumps(payload).encode("utf-8"))


class UploadLanzouTests(unittest.TestCase):
    def test_current_html5_upload_endpoint_is_pinned(self):
        self.assertEqual(
            upload_module.UPLOAD_URL, "https://pc.woozooo.com/html5up.php"
        )

    def create_release_files(self, root: Path) -> tuple[Path, Path, str]:
        installer = root / "VALOFRAME-0.2.5-x64-setup.exe"
        installer.write_bytes(b"verified installer bytes")
        digest = hashlib.sha256(installer.read_bytes()).hexdigest()
        checksums = root / "SHA256SUMS.txt"
        checksums.write_text(f"{digest}  {installer.name}\n", encoding="utf-8")
        return installer, checksums, digest

    def test_cookie_requires_both_login_values(self):
        with self.assertRaisesRegex(upload_module.LanzouUploadError, "phpdisk_info"):
            upload_module.parse_cookie("ylogin=123")

    def test_new_installer_is_uploaded_and_hash_marked(self):
        with tempfile.TemporaryDirectory() as directory:
            installer, checksums, digest = self.create_release_files(Path(directory))
            transport = FakeTransport(
                [
                    upload_module.HttpResponse(200, "我的网盘".encode("utf-8")),
                    json_response(
                        {
                            "info": {
                                "name": "VALOFRAME",
                                "new_url": "https://example.lanzoue.com/b12345678",
                                "onof": "1",
                                "pwd": "4sj6",
                            }
                        }
                    ),
                    json_response({"info": 0, "text": []}),
                    json_response({"zt": 1, "text": [{"id": "321"}]}),
                    json_response({"zt": 1}),
                ]
            )

            result = upload_module.synchronize(
                file_path=installer,
                checksums_path=checksums,
                tag="v0.2.5",
                repository="2424521842/valoframe",
                folder_id=42,
                cookie="ylogin=123; phpdisk_info=secret",
                transport=transport,
            )

            self.assertEqual(result["status"], "uploaded")
            self.assertEqual(result["file"]["sha256"], digest)
            self.assertEqual(result["folder"]["password"], "4sj6")
            upload_request = transport.requests[3]
            self.assertEqual(upload_request[1], upload_module.UPLOAD_URL)
            self.assertIn(installer.name.encode("utf-8"), upload_request[3])
            description_form = urllib.parse.parse_qs(transport.requests[4][3].decode("ascii"))
            self.assertEqual(description_form["task"], ["11"])
            self.assertIn(digest, description_form["desc"][0])
            self.assertEqual(transport.responses, [])

    def test_matching_hash_marker_makes_rerun_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            installer, checksums, digest = self.create_release_files(Path(directory))
            description = f"VALOFRAME v0.2.5 SHA-256: {digest}"
            transport = FakeTransport(
                [
                    upload_module.HttpResponse(200, "我的网盘".encode("utf-8")),
                    json_response(
                        {
                            "info": {
                                "name": "VALOFRAME",
                                "new_url": "https://example.lanzoue.com/b12345678",
                                "onof": "0",
                                "pwd": "ignored",
                            }
                        }
                    ),
                    json_response(
                        {
                            "info": 1,
                            "text": [{"id": "321", "name_all": installer.name}],
                        }
                    ),
                    json_response({"info": 0, "text": []}),
                    json_response({"zt": 1, "info": description, "text": installer.stem}),
                ]
            )

            result = upload_module.synchronize(
                file_path=installer,
                checksums_path=checksums,
                tag="v0.2.5",
                repository="2424521842/valoframe",
                folder_id=42,
                cookie="ylogin=123; phpdisk_info=secret",
                transport=transport,
            )

            self.assertEqual(result["status"], "already-present")
            self.assertNotIn(upload_module.UPLOAD_URL, [request[1] for request in transport.requests])

    def test_same_name_without_hash_marker_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            installer, checksums, _digest = self.create_release_files(Path(directory))
            transport = FakeTransport(
                [
                    upload_module.HttpResponse(200, "我的网盘".encode("utf-8")),
                    json_response(
                        {
                            "info": {
                                "name": "VALOFRAME",
                                "new_url": "https://example.lanzoue.com/b12345678",
                                "onof": "0",
                            }
                        }
                    ),
                    json_response(
                        {"info": 1, "text": [{"id": "321", "name_all": installer.name}]}
                    ),
                    json_response({"info": 0, "text": []}),
                    json_response({"zt": 1, "info": "different file"}),
                ]
            )

            with self.assertRaisesRegex(upload_module.LanzouUploadError, "refusing to overwrite"):
                upload_module.synchronize(
                    file_path=installer,
                    checksums_path=checksums,
                    tag="v0.2.5",
                    repository="2424521842/valoframe",
                    folder_id=42,
                    cookie="ylogin=123; phpdisk_info=secret",
                    transport=transport,
                )


class ReleaseNotesTests(unittest.TestCase):
    def result(self):
        return {
            "schemaVersion": 1,
            "status": "uploaded",
            "file": {
                "name": "VALOFRAME-0.2.5-x64-setup.exe",
                "sha256": "a" * 64,
            },
            "folder": {
                "url": "https://example.lanzoue.com/b12345678",
                "password": "4sj6",
            },
        }

    def test_mirror_is_inserted_inside_download_section(self):
        block = notes_module.build_block(self.result())
        updated = notes_module.update_notes(
            "# v0.2.5\n\n## 下载\n\nGitHub link\n\n## 修复\n\nDetails\n", block
        )
        self.assertLess(updated.index("GitHub link"), updated.index(notes_module.START_MARKER))
        self.assertLess(updated.index(notes_module.END_MARKER), updated.index("## 修复"))
        self.assertIn("提取码：`4sj6`", updated)

    def test_existing_mirror_block_is_replaced_not_duplicated(self):
        first = notes_module.build_block(self.result())
        updated_once = notes_module.update_notes("# Release\n", first)
        changed = self.result()
        changed["folder"]["password"] = "abcd"
        updated_twice = notes_module.update_notes(updated_once, notes_module.build_block(changed))
        self.assertEqual(updated_twice.count(notes_module.START_MARKER), 1)
        self.assertIn("提取码：`abcd`", updated_twice)


if __name__ == "__main__":
    unittest.main()
