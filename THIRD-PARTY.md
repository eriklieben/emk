# Third-party licenses

EMK's own code is MIT ([LICENSE](LICENSE)). These parts are not EMK's and keep their own licenses.

## Nordic SoftDevice Controller and MPSL (Bluetooth builds of the firmware)

`BLE=1` builds of `fw-nrf` link Nordic's SoftDevice Controller and MPSL libraries, through the
`nrf-sdc` / `nrf-sdc-sys` and `nrf-mpsl` / `nrf-mpsl-sys` crates. These binaries are under
**LicenseRef-Nordic-5-Clause**:

- they may only be used with (or in a product built around) a Nordic Semiconductor integrated
  circuit, such as the nice!nano's nRF52840;
- whoever redistributes them in binary form outside a device (for example a `.uf2`, `.hex` or `.zip`
  firmware file published for download) must reproduce Nordic's copyright notice, the conditions
  and the disclaimer in the documentation or other materials that come with it.

The full text ships inside those crates at `third_party/nordic/nrfxlib/LICENSE` (in the Cargo
registry, for example `~/.cargo/registry/src/*/nrf-sdc-sys-*/third_party/nordic/nrfxlib/LICENSE`,
or `.cargo-home/registry/...` when built with `./dev`). Put a copy of it next to any firmware file
you publish. The nrfx parts in the same crates (`third_party/nordic/nrfx`) are BSD-3-Clause, and
the Arm CMSIS headers (`third_party/arm/CMSIS_5`) are Apache-2.0.

USB-only builds (`./fw-nrf/build-uf2` without `BLE=1`) don't contain these libraries.

## Fonts (emulator app)

The app bundles these fonts, all under the SIL Open Font License 1.1. The license texts are next to
them in `emulator/App/Assets/Fonts/`:

| Font | License file |
|---|---|
| Barlow | `OFL-Barlow.txt` |
| Barlow Semi Condensed | `OFL-BarlowSemiCondensed.txt` |
| IBM Plex Mono | `OFL-IBMPlexMono.txt` |
| Noto Sans Symbols 2 | `OFL-NotoSansSymbols2.txt` |

## Rust and .NET dependencies

The Rust crates (Embassy, trouble-host, embedded-graphics, sequential-storage and the rest, see
`Cargo.lock` and `fw-nrf/Cargo.lock`) and the NuGet packages (Avalonia, TUnit) keep their own
license terms; each package carries its license. Nearly all are MIT and/or Apache-2.0. The
exceptions are permissive too: BSD-3-Clause (`nrf-pac`, `subtle`), Zlib (`zlib-rs`, host tools
only), and Unicode-3.0 in addition to MIT/Apache-2.0 (`unicode-ident`, used at build time by
proc macros). `SonarAnalyzer.CSharp` (LGPL-3.0) is a build-time code analyzer and is not part of
the app.
