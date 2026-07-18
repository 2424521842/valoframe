# FFmpeg corresponding source status

This application invokes FFmpeg as a separate command-line program. The bundled
Windows x64 executable is the unmodified `win64-lgpl` static artifact described
in `BUILD-INFO.json` and is licensed under LGPL-3.0-or-later.

Upstream traceability:

- FFmpeg commit: `ce3c09c101c83add623774d414a9f9498caf5c25`
- FFmpeg reference source: https://github.com/FFmpeg/FFmpeg/archive/ce3c09c101c83add623774d414a9f9498caf5c25.tar.gz
- BtbN build scripts commit: `7a83528ea3431e9eca982a712bc3a7cd0789d5d0`
- Build scripts reference source: https://github.com/BtbN/FFmpeg-Builds/archive/7a83528ea3431e9eca982a712bc3a7cd0789d5d0.tar.gz
- Provider release: https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-06-30-13-34
- Redistribution status: `blocked-pending-corresponding-source-mirror-and-third-party-license-review`

## Release gate: not yet a complete source offer

The URLs above are traceability references, not a durable corresponding-source
offer. Public redistribution remains blocked until the release owner:

1. creates one immutable, checksum-pinned source bundle containing the exact
   FFmpeg source, BtbN build scripts and patches, and source for every statically
   linked dependency;
2. publishes that bundle beside the application installer under a stable URL;
3. records its URL, byte length and SHA-256 in
   `third_party/ffmpeg/windows-x64.json`;
4. completes the third-party license inventory, IJG attribution, and applicable
   codec patent review; and
5. replaces this section with the final corresponding-source availability terms
   approved for the release.

The release preparation and verification scripts reject redistribution mode
while the manifest's `sourceCompliance.redistributionReady` value is false.
