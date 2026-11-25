# Vendored patch: ffmpeg-sys-next 9.0.0 (FFmpeg 9 / CUDA bindgen)

This directory is the crates.io release **ffmpeg-sys-next 9.0.0**, unmodified
except for one hunk in `hwcontext_wrapper.h`. It is wired in through
`[patch.crates-io]` in the repository `Cargo.toml` so that a plain
`cargo build --release --locked` works against FFmpeg 9 headers.

## Provenance

| Item | Value |
|---|---|
| Crate | `ffmpeg-sys-next` |
| Version | `9.0.0` |
| Source | crates.io registry tarball `ffmpeg-sys-next-9.0.0.crate` |
| SHA-256 (`.crate`) | `9b939bf79dd5949412a4b81cfe21a07f48ea21b47fcbb5f57816c8c2de5ae30b` |
| Upstream repository | <https://github.com/zmwangx/rust-ffmpeg-sys> |
| Upstream commit (`Cargo.toml`/tarball) | `80b7dd8327c3539159f37d8ca5423c75d6cd2e57` |
| License | WTFPL (declared in `Cargo.toml`; the crate ships no separate LICENSE file) |

Local extraction used as the diff base:

`~/.cargo/registry/src/index.crates.io-*/ffmpeg-sys-next-9.0.0`

## The only change

`hwcontext_wrapper.h`, inside the CUDA block (before
`#include <libavutil/hwcontext_cuda.h>`), adds the types that FFmpeg 9's
`libavutil/hwcontext_cuda.h` now uses:

```diff
 typedef struct CUctx_st *CUcontext;
 typedef struct CUstream_st *CUstream;
+typedef struct CUarray_st *CUarray;
+// AVCUDAFramesContext holds this by value, so it needs cuda.h's exact layout.
+typedef int CUarray_format;
+typedef struct CUDA_ARRAY3D_DESCRIPTOR_st {
+    size_t Width;
+    size_t Height;
+    size_t Depth;
+    CUarray_format Format;
+    unsigned int NumChannels;
+    unsigned int Flags;
+} CUDA_ARRAY3D_DESCRIPTOR;
 #include <libavutil/hwcontext_cuda.h>
```

This is the hunk from upstream PR
<https://github.com/zmwangx/rust-ffmpeg-sys/pull/139>, applied verbatim.

Without it, bindgen stops at:

```
/usr/include/libavutil/hwcontext_cuda.h:66:5: error: unknown type name 'CUarray'
```

(`AVCUDAArrayFrameDescriptor` uses `CUarray`, and `AVCUDAFramesContext` embeds
`CUDA_ARRAY3D_DESCRIPTOR` by value, so an opaque forward declaration alone is
not enough.)

Everything else in this directory is byte-identical to the registry crate.
The following non-source files from the tarball were deliberately **not**
vendored: `.cargo-ok`, `.gitignore`, `.github/`, `Cargo.lock` and
`Cargo.toml.orig`. `Cargo.toml` is the cargo-normalized manifest, kept as
published; it has no `path` dependencies, so it works unchanged as a path
dependency.

## Removal condition

Delete this directory and the `[patch.crates-io]` section in the repository
`Cargo.toml` as soon as a crates.io release of `ffmpeg-sys-next` newer than
`9.0.0` includes the fix (upstream PR #139 or an equivalent). Then run:

```sh
cargo update -p ffmpeg-sys-next
```

and verify `cargo build --release --locked` still succeeds.
