# Xrayc

A simple **Electron + TypeScript** desktop client for **Xray-core**, with a small **Rust network service**.

Paste a configuration, press **Connect**, and press **Disconnect** when you’re done. The interface is just a configuration editor, optional encrypted storage, one connection button, and status.

## Your configuration decides the mode

**Inbounds are used exactly as provided.** The application does not insert TUN, replace listeners, change ports, configure a system proxy, override outbounds, or rewrite DNS/routing rules.

- A `tun` inbound uses **Xray’s own native TUN implementation**.
- SOCKS/HTTP inbounds run at the addresses/ports you configured. Configure your applications to use them; they do not automatically become a system-wide VPN.
- Mixed inbounds and direct outbounds are allowed.
- There is **no sing-box or TUN-to-SOCKS bridge**.

Xray-core **26.11.9** is bundled with the application, together with its GeoIP/GeoSite assets and platform-specific Wintun DLL where supplied by the Windows release. There are no runtime binary downloads.

## Use it

1. Open **Xrayc** from the application menu, or run `xrayc`.
2. Paste a complete Xray client JSON configuration. Comments and trailing commas are accepted. Subscription URLs and `vless://` / `vmess://` links are not configuration files.
3. Press **Connect**. A configuration with TUN or privileged networking settings will request administrator approval. A normal SOCKS/HTTP listener on a high port runs without elevation.
4. Press **Disconnect** to stop Xray. Closing the application also disconnects.

The small line below the editor shows the configured inbound/outbound protocols and whether authorization is required. Preliminary validation checks the JSON structure; Xray itself checks the configuration before starting any inbounds. **Connected** means Xray has started, not that a remote server’s credentials/reachability have been independently verified. There are no analytics or external connectivity probes.

**Remember config** is off by default. Saved configurations use Electron’s OS-backed `safeStorage`. Saving is disabled when secure storage is unavailable, including Linux’s insecure `basic_text` backend; unlock/configure your desktop keyring to enable it. Runtime configurations briefly exist in private temporary directories so Xray can read them. Normal shutdown removes them; a hard service/system crash may leave a private temporary directory behind.

### Native TUN configuration

Simply choosing `"protocol": "tun"` does **not** guarantee a transparent, full-system VPN. Interface addresses, routes, outbound-loop prevention, DNS and bypass rules must be configured appropriately in your pasted file. Xrayc intentionally leaves these decisions alone.

The bundled release supports settings including `gateway`, `autoSystemRoutingTable`, `autoOutboundsInterface` and platform-dependent DNS options. For example, a user-selected split-default route list can be:

```json
{
  "protocol": "tun",
  "tag": "tun",
  "settings": {
    "name": "utun10",
    "gateway": ["172.28.233.1/30", "fd75:7872:6179::1/126"],
    "autoSystemRoutingTable": ["0.0.0.0/1", "128.0.0.0/1", "::/1", "8000::/1"],
    "autoOutboundsInterface": "auto"
  }
}
```

This is an **inbound snippet**, not a complete working VPN configuration. You still need your proxy outbounds and an appropriate DNS policy. In particular, Linux’s `autoSystemDnsToGateway` requires a compatible running systemd-resolved and a correctly routed DNS outbound; macOS does not configure system DNS automatically. Do not enable options blindly.

See the pinned upstream [native TUN documentation](https://github.com/XTLS/Xray-core/blob/v26.11.9/proxy/tun/README.md) for platform-specific behavior and requirements.

## Security and limitations

- The Electron renderer is sandboxed, context isolated, and has no Node.js integration. Only a small preload API is exposed.
- Only configurations detected as needing privileged networking are elevated: native TUN, Unix low ports, transparent proxy settings, or outbound packet marks. Other unusual OS-specific permission requirements may still cause an explicit Xray startup error.
- Linux authorization uses `pkexec`, falling back to `sudo -A` only if `pkexec` is unavailable. macOS uses its administrator dialog; Windows uses UAC.
- The network service and Xray session communicate over an authenticated, heartbeat-monitored loopback connection. Closing it stops Xray. Linux parent-death signals also stop Xray if the session process is killed.
- **The only runtime configuration change is logging:** imported log-file paths are replaced with console diagnostics at info level, with access logging disabled. This avoids privileged writes to arbitrary imported paths and allows reliable startup detection. All networking fields, including API inbounds and routing, remain unchanged. Diagnostics are not written to a persistent application log.
- Import only configurations you trust. Inbounds may expose ports beyond localhost if the supplied configuration requests it. Relative certificate paths resolve from the bundled binary directory; use absolute paths when appropriate.
- This is **not an app-enforced kill switch** or an independently audited leak-prevention product. Routing, DNS protection and bypass behavior depend on your configuration and Xray’s platform support.
- **Linux x64 is built and tested locally.** Windows/macOS paths and x64/arm64 downloads are implemented but still require native end-to-end testing and signed release builds. Windows session termination uses process termination and has not been verified to clean up every native TUN/DNS setting; do not treat those targets as production validated.
- There is no auto-start, automatic reconnection, or automatic update service. Public Windows/macOS distribution additionally needs signing/notarization and platform validation.

## Development

Requirements: a recent stable Rust toolchain, Node.js **22.12+** (developed with Node 26), and the target platform’s desktop build tools. Linux needs Electron’s GTK/NSS/audio dependencies and polkit with an authentication agent for elevated configurations. Core setup uses `unzip`/`tar` on Unix and PowerShell on Windows.

```sh
npm ci
npm run prepare:cores   # Download pinned, SHA-256-verified Xray release
npm run build:service
npm run dev
```

Restart the development app after changing main/preload code. Download URLs, versions and SHA-256 hashes are recorded in [`scripts/cores.json`](scripts/cores.json).

### Test

```sh
npm test
cargo clippy --manifest-path native/Cargo.toml --all-targets --locked -- -D warnings
npm run test:cores
npm run build
npm run test:ui
npm audit
```

Linux integration tests require `iproute2`, Python 3, `unshare`, `mount`, `/dev/net/tun`, and enabled unprivileged user namespaces. They isolate the **user, mount and network namespaces**, hide host D-Bus sockets, and use a local upstream fixture. Tests exercise real **native Xray TUN IPv4/IPv6 TCP, UDP and explicit DNS traffic**, clean disconnect, startup cancellation, process crashes and the connection manager. They do not change your host’s routes/DNS or require a real proxy server/admin prompt. System resolver takeover itself is not exercised.

Desktop tests exercise real renderer/preload/IPC and configuration validation, the minimal layout and sandbox settings, and a **real non-elevated SOCKS connection**. A separate state-rendering test simulates activation; native TUN is tested only inside the isolated network namespace.

### Package and install

```sh
npm run package
# Linux, after testing passes:
npm run install:local
```

The Linux installer requires no elevated privileges. It installs to `~/.local/opt/xrayc`, links `~/.local/bin/xrayc`, and adds `~/.local/share/applications/xrayc.desktop` and an icon. Re-running it replaces the installation without changing preferences.

Test a packaged/installed executable:

```sh
XRAYC_TEST_BINARY="$HOME/.local/opt/xrayc/xrayc" npm run test:ui
```

To uninstall, remove those paths and `~/.local/share/icons/hicolor/256x256/apps/xrayc.png`. Remove the per-user Xrayc application-data directory separately to delete remembered settings.

## Source layout

- `src/renderer/` — minimal editor and connection state.
- `src/main/` — Electron integration, encrypted preferences and connection transport.
- `native/src/config.rs` — validation and permission detection; networking settings are preserved.
- `native/src/controller.rs` — authorization and connection lifecycle.
- `native/src/main.rs` — session-owned Xray process and cleanup.
- `scripts/` — builds, downloads, packaging and installation.
- `tests/` — desktop and isolated native TUN tests.

Xrayc is MIT licensed. Bundled Xray-core is MPL-2.0; platform driver and dependency licenses remain applicable. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
