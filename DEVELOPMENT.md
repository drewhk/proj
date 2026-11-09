# Development related notices

## Bundled native PROJ

`proj-sys/PROJSRC/proj` is a Git submodule pinned to an exact native PROJ
commit. It replaces the source tarball; the initial pin is the unmodified
upstream `9.6.0` release. Native PROJ and the Rust wrapper can be updated
independently.

For a local checkout, initialize sources before building:

```sh
git submodule update --init --recursive
cargo test --lib --features bundled_proj,tiff
```

Cargo fetches submodules automatically when using this repository as a Git
dependency. Builds use the recorded commit, never the remote branch tip.

To update native PROJ, fetch and select an explicit stable release tag in
the submodule, then commit its new pin in this repository. Do not track
upstream master or select a release automatically during builds. Once the
submodule points at our native fork, integrate the chosen upstream release
there first and pin the tested fork commit here.

Review native build/header changes and run upstream, extension and application
conformance tests before accepting an upgrade. Keep the existing SQLite,
TIFF/zlib, Android and static-link settings. Static PROJ currently embeds its
matching `proj.db`. An upgrade does not require merging upstream Rust-wrapper
changes, but may require local build adjustments. `MINIMUM_PROJ_VERSION` is
the system-library compatibility floor, not the bundled source selection.

The app and GDAL must continue to resolve to one `proj-sys` dependency. After
updating the wrapper pin, update the application's Cargo lockfile as well.

## Bindings

Bindings were generated with the following command:

```sh
bindgen --blocklist-type max_align_t wrapper.h -- -I PROJSRC/proj/src
```

If you update the above command line you also need to update the arguments for the buildtime_bindgen feature in `build.rs`
