# RecapIt patch for whisper-rs-sys 0.15.0

The source baseline is the crates.io `whisper-rs-sys` 0.15.0 archive with SHA-256 `6986c0fe081241d391f09b9a071fbcbb59720c3563628c3c829057cf69f2a56f`. Its `.cargo_vcs_info.json` identifies wrapper revision `7558e1b72f54f2f22a53589afb77e65681834c36` at path `sys`. That wrapper revision records the `whisper.cpp` submodule at `2eeeba56e9edd762b4b38467bab96c2517163158` (upstream release v1.8.3). The crate's included whisper.cpp files match that submodule revision byte for byte before this patch.

The fork records the crate's included whisper.cpp source as regular files under `sys/whisper.cpp`, replacing the upstream gitlink. Cargo can fetch the complete wrapper and native sources from one immutable revision, and offline graph checks do not rely on a nested repository checkout.

## Build source synchronization

Cargo now reruns the build script when any file or directory under `whisper.cpp` changes. Before CMake configuration, the std-only synchronizer compares the complete checked-in source tree with `OUT_DIR/whisper.cpp`, copies changed or new files, and removes stale files and directories. Identical files remain untouched. It reconciles file-to-directory and directory-to-file changes; destination presence is checked with `symlink_metadata`, and destination symlinks (including dangling links) are unlinked without following their targets. This replaces the old `if !whisper_root.exists()` copy guard, which could reuse stale or partially populated output on later builds. The build script and a native-free `source_sync` Rust test use the same helper; the test covers type transitions, dangling links, and no-change mtime preservation.
 The fixture allocator uses an atomic per-process sequence and exclusive directory creation so parallel tests cannot share or remove each other's temporary trees.

## DTW median-filter behavior

The production behavior change is limited to Whisper's DTW median filter: windows with fewer audio tokens than the normal width-7 median use the largest fitting odd width, including width 1 for a singleton axis. The width-7 result is unchanged for axes with at least eight tokens. DTW, token timestamps, and word timing remain enabled.

This follows the causal case and algorithm in whisper.cpp commit `70fdf1c5b6a3a30653b3b400933da1c4cea30a4d`, from [PR #4096](https://github.com/ggml-org/whisper.cpp/pull/4096), merged on 2026-10-19. The wrapper's frozen submodule baseline predates that merge, so the fork applies the behavior locally. The local implementation factors the filter into a callback-based header so a dependency-free C++ fixture compares production outputs with an independent reference. It also accepts ggml's 2D representation of a singleton third axis; input validation still requires one fourth axis and F32 input. `UPSTREAM-PR-4096.patch` preserves the upstream patch transcript.
