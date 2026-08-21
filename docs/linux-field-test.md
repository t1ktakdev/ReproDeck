# Linux desktop field-test checklist

Linux runtime packaging was not executed for the Windows 0.3.0 release pass. Use this checklist before claiming support for a distribution/compositor combination.

## Build matrix

- Arch Linux and one Debian/Ubuntu-family distribution;
- Wayland under KDE Plasma;
- Wayland under a wlroots compositor such as Niri;
- X11 fallback where available.

Install the current Tauri 2 Linux prerequisites for the distribution, then run:

```bash
npm ci
npm run typecheck
npm test
npm run build
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
npm run tauri dev
```

## Runtime checks

- native directory/file dialogs open and return Unicode paths;
- Ctrl+K, Ctrl+N, Ctrl+O, Ctrl+B, Ctrl+Shift+I and Ctrl+, work;
- independent rail/sidebar/main/inspector scrolling at 1366×768 and 150% text/UI scaling;
- custom Select, dialogs and focus-visible work entirely from the keyboard;
- the deterministic demo produces check PASS / test FAIL / build PASS;
- Fix Workspace and verification worktrees remain under the OS temporary directory;
- Investigation patch transfer occurs only after failing Before;
- mutation after After blocks Apply and rerunning verification restores it;
- original HEAD/index/worktree remain unchanged before explicit Apply;
- AppImage/deb/rpm launch behavior and desktop integration are recorded separately for each artifact.

Do not infer Wayland/Niri/KDE runtime support from successful Rust or frontend compilation alone.
