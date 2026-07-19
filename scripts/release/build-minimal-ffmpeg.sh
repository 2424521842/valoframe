#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <ffmpeg-source-directory> <candidate-manifest> <output-directory>" >&2
  exit 64
fi

source_dir="$(realpath "$1")"
manifest_path="$(realpath "$2")"
output_dir="$3"

if [[ ! -d "$source_dir/.git" ]]; then
  echo "FFmpeg source must be an exact Git checkout: $source_dir" >&2
  exit 65
fi
if [[ -e "$output_dir" ]]; then
  echo "Output directory must not already exist: $output_dir" >&2
  exit 65
fi

expected_commit="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["source"]["commit"])' "$manifest_path")"
actual_commit="$(git -C "$source_dir" rev-parse HEAD)"
if [[ "$actual_commit" != "$expected_commit" ]]; then
  echo "FFmpeg source commit mismatch: expected $expected_commit, found $actual_commit" >&2
  exit 66
fi
if [[ -n "$(git -C "$source_dir" status --porcelain --untracked-files=all)" ]]; then
  echo "FFmpeg source checkout must be clean." >&2
  exit 66
fi

mapfile -t configure_flags < <(
  python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1], encoding="utf-8"))["build"]["configureFlags"]))' "$manifest_path"
)
if [[ ${#configure_flags[@]} -lt 20 ]]; then
  echo "Candidate manifest contains an unexpectedly small configure contract." >&2
  exit 67
fi
for flag in "${configure_flags[@]}"; do
  if [[ "$flag" == --enable-gpl || "$flag" == --enable-nonfree || "$flag" == --enable-lib* ]]; then
    echo "Candidate manifest enables a forbidden external/GPL option: $flag" >&2
    exit 67
  fi
done
commit_version_id="${expected_commit:0:12}"
if [[ "${configure_flags[*]}" != *"--extra-version="*"$commit_version_id"* ]]; then
  echo "Candidate extra-version must identify the pinned commit with 12 hexadecimal characters." >&2
  exit 67
fi

output_dir="$(realpath -m "$output_dir")"
mkdir "$output_dir"
build_dir="$output_dir/build"
stage_dir="$output_dir/stage"
mkdir "$build_dir" "$stage_dir"

source_date_epoch="$(git -C "$source_dir" show -s --format=%ct HEAD)"
export SOURCE_DATE_EPOCH="$source_date_epoch"
export LC_ALL=C
export TZ=UTC

pushd "$build_dir" >/dev/null
"$source_dir/configure" \
  "--prefix=$stage_dir" \
  "${configure_flags[@]}" \
  >"$output_dir/configure.stdout.txt" \
  2>"$output_dir/configure.stderr.txt"
make -j"${FFMPEG_BUILD_JOBS:-$(nproc)}" V=1 \
  >"$output_dir/build.stdout.txt" \
  2>"$output_dir/build.stderr.txt"
make install \
  >"$output_dir/install.stdout.txt" \
  2>"$output_dir/install.stderr.txt"
popd >/dev/null

candidate="$stage_dir/bin/ffmpeg.exe"
if [[ ! -s "$candidate" ]]; then
  echo "Minimal build did not produce ffmpeg.exe." >&2
  exit 68
fi

cp "$candidate" "$output_dir/ffmpeg.exe"
cp "$build_dir/config.h" "$output_dir/config.h"
cp "$build_dir/ffbuild/config.mak" "$output_dir/config.mak"
printf '%s\n' "${configure_flags[@]}" >"$output_dir/configure-flags.txt"
x86_64-w64-mingw32-gcc --version >"$output_dir/compiler-version.txt"
x86_64-w64-mingw32-objdump -p "$candidate" >"$output_dir/pe-imports.txt"
sha256sum "$output_dir/ffmpeg.exe" >"$output_dir/ffmpeg.exe.sha256"

python3 - "$manifest_path" "$output_dir" "$actual_commit" "$source_date_epoch" <<'PY'
import hashlib
import json
import pathlib
import sys

manifest_path, output_path, commit, epoch = sys.argv[1:]
output = pathlib.Path(output_path)
binary = output / "ffmpeg.exe"
manifest = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
metadata = {
    "schemaVersion": 1,
    "status": "built-candidate-not-promoted",
    "sourceCommit": commit,
    "sourceDateEpoch": int(epoch),
    "targetTriple": manifest["build"]["targetTriple"],
    "licenseExpression": manifest["build"]["licenseExpression"],
    "configureFlags": manifest["build"]["configureFlags"],
    "externalLibraries": manifest["build"]["externalLibraries"],
    "executable": {
        "fileName": binary.name,
        "sizeBytes": binary.stat().st_size,
        "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    },
}
(output / "BUILD-METADATA.json").write_text(
    json.dumps(metadata, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY

echo "Minimal FFmpeg candidate built at $output_dir/ffmpeg.exe"
