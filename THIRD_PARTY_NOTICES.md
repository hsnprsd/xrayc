# Bundled components

## Xray-core 26.11.9

Xrayc embeds the unmodified upstream Go packages in a shared library, loaded into the Rust network-service process. The application-owned C ABI wrapper is MIT licensed; the upstream Xray source remains MPL-2.0.

- Copyright: XTLS contributors and respective upstream authors.
- License: Mozilla Public License 2.0.
- Release: https://github.com/XTLS/Xray-core/releases/tag/v26.11.9
- Corresponding source and build materials: https://github.com/XTLS/Xray-core/tree/v26.11.9
- Source archive: https://github.com/XTLS/Xray-core/archive/refs/tags/v26.11.9.tar.gz
- Exact source commit: `676eaf66f8f05d4d558ee7a74b4736623d3feeb9`.
- Reproducible source inputs: `native/xray/go.mod`, `native/xray/go.sum`, and `scripts/build-core.mjs`.
- Bundled license: `resources/bin/xray-LICENSE`.
- Go runtime and linked-module license notices: `resources/bin/xray-dependency-licenses.txt`, generated from the build's source dependencies.

Xray’s GeoIP/GeoSite assets and third-party dependencies retain their upstream licenses; see the pinned release and source for provenance and notices. Windows releases include Wintun; its upstream license is copied next to the binary as `xray-LICENSE-Wintun` when present in the archive.

## Electron and dependencies

Electron/Chromium license notices ship with the Electron distribution. JavaScript and Rust dependencies retain their respective licenses. Exact dependency versions are recorded in `package-lock.json`, `native/Cargo.lock`, and `native/xray/go.sum`; consult their upstream sources for redistribution notices.

This notice does not replace upstream licenses or a redistributor’s obligation to provide notices, source or other materials when required.
