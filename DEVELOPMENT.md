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

Enable `native_extensions` on `proj-sys` to generate the local C ABI
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

## Rust interpolation extensions

`proj-extensions` depends only on `proj-sys`, enabling its native extension ABI.
It contains cubic Lagrange, CNC bicubic and nearest-neighbour interpolation,
with explicit registration and a small native registration error type.
It does not depend on the high-level `proj` crate. Native PROJ remains owned
and built by `proj-sys`; the app and GDAL must resolve to that same package.

## Cubic Lagrange conformance

`proj_extensions::register_cubic_lagrange()` registers
`rust_cubic_lagrange` for explicit gridshift interpolation or opaque sampling.
It requires the complete floor(index)-1 through floor(index)+2 window on both
axes, and rejects nodata or missing stencil nodes. There is no edge fallback;
the Czech authority's boundary policy is still unverified.

The isolated Czech tests use the app's preserved authority captures and grids,
not copied fixture data in this wrapper. They are explicitly ignored in ordinary
wrapper runs because that external test-data directory is required. Run them as:

```powershell
python <app-checkout>/test_data/czechia/prepare_interpolation_data.py
$env:PROJ_CZECH_DATA = '<app-checkout>/test_data'
cargo test -p proj-extensions --features tiff --lib
cargo test -p proj-extensions --features tiff --test czech_interpolation -- --ignored --nocapture
```

The preparation verifies capture and grid hashes. Both reference tests require
zero millimetre-rounded mismatch counts; raw maxima are reported for inspection.
The horizontal test also checks the mathematical inverse. These tests exclude
the Czech Helmert/projection steps and do not establish full-pipeline conformance.

## Bindings

Bindings were generated with the following command:

```sh
cd proj-sys
bindgen --blocklist-type max_align_t wrapper.h -- -I PROJSRC/proj/src -DPROJ_SYS_NATIVE_EXTENSIONS
```

Omit the define for ordinary upstream bindings. Normal Cargo builds generate
bindings automatically in `OUT_DIR`; do not check those generated files in.
Keep manual bindgen arguments consistent with `proj-sys/build.rs`.
