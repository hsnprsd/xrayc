# Bundled components

## Xray-core 26.11.9

Xrayc invokes the unmodified upstream command-line binary as a separate process.

- Copyright: XTLS contributors and respective upstream authors.
- License: Mozilla Public License 2.0.
- Release: https://github.com/XTLS/Xray-core/releases/tag/v26.11.9
- Corresponding source and build materials: https://github.com/XTLS/Xray-core/tree/v26.11.9
- Source archive: https://github.com/XTLS/Xray-core/archive/refs/tags/v26.11.9.tar.gz
- Bundled license: `resources/bin/xray-LICENSE`.

Xray’s GeoIP/GeoSite assets and third-party dependencies retain their upstream licenses; see the pinned release and source for provenance and notices. Windows releases include Wintun; its upstream license is copied next to the binary as `xray-LICENSE-Wintun` when present in the archive.

## Electron and dependencies

Electron/Chromium license notices ship with the Electron distribution. JavaScript and Rust dependencies retain their respective licenses. Exact dependency versions are recorded in `package-lock.json` and `native/Cargo.lock`; consult their upstream sources for redistribution notices.

This notice does not replace upstream licenses or a redistributor’s obligation to provide notices, source or other materials when required.
