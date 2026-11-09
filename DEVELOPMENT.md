# Development related notices

## Bundled native PROJ

`proj-sys/PROJSRC/proj` is a Git submodule pinned to an exact native PROJ
commit from [our native fork](https://github.com/drewhk/proj-native).
The fork is based on upstream `9.6.0`, with `somerc +latc` and the local
extension ABI. Native PROJ and the Rust wrapper can be updated independently.

For a local checkout, initialize sources before building:

```sh
git submodule update --init --recursive
cargo test --lib --features bundled_proj,tiff
```

Cargo fetches submodules automatically when using this repository as a Git
dependency. Builds use the recorded commit, never the remote branch tip.

To update native PROJ, fetch an explicit stable release tag from
`https://github.com/OSGeo/PROJ.git` in the submodule and integrate that release
into our fork branch. Do not track upstream master or select a release
automatically during builds. Test and push the native fork commit first,
then commit its new submodule pin here. This preserves our patches instead
of replacing the fork with an unmodified upstream tag.

Review native build/header changes and run upstream, extension and application
conformance tests before accepting an upgrade. Keep the existing SQLite,
TIFF/zlib, Android and static-link settings. Static PROJ currently embeds its
matching `proj.db`. An upgrade does not require merging upstream Rust-wrapper
changes, but may require local build adjustments. `MINIMUM_PROJ_VERSION` is
the system-library compatibility floor, not the bundled source selection.

The app and GDAL must continue to resolve to one `proj-sys` dependency. After
updating the wrapper pin, update the application's Cargo lockfile as well.

## Native extension bindings

Enable `native_extensions` on `proj` or `proj-sys` to generate the local C ABI
bindings in `proj-sys`. The feature implies `bundled_proj`: a system PROJ cannot
provide these symbols. Without it, the ordinary system-library path and public
bindings remain unchanged. Grid sampling additionally requires `tiff`.

These are unsafe low-level bindings, not a new safe callback wrapper. Follow
the ownership, thread, coordinate-domain and no-unwind contracts in
[`proj_extensions.h`](proj-sys/PROJSRC/proj/src/proj_extensions.h).
Register callbacks once before creating pipelines; callback code must remain
loaded. The native adapter owns each instance and reconstructs it when cloned.
Country algorithms and application startup registration are separate work.

The Rust integration tests register Rust callbacks, run forward/inverse and
cloning through the linked native fork, and exercise opaque grid sampling:

```sh
cargo test -p proj-sys --features native_extensions,tiff --test extensions
```

## Bindings

Bindings were generated with the following command:

```sh
cd proj-sys
bindgen --blocklist-type max_align_t wrapper.h -- -I PROJSRC/proj/src -DPROJ_SYS_NATIVE_EXTENSIONS
```

Omit the define for ordinary upstream bindings. Normal Cargo builds generate
bindings automatically in `OUT_DIR`; do not check those generated files in.
Keep manual bindgen arguments consistent with `proj-sys/build.rs`.
