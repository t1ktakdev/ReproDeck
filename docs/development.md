# Development

## Windows preflight

```powershell
node --version
npm --version
rustc --version
cargo --version
git --version
```

Then:

```powershell
npm install --no-audit
.\scripts\verify-windows.ps1 -SkipInstall
npm run tauri dev
```

The verification script is the local quality gate. A production release should not be created from a tree that fails it.

## Deterministic acceptance fixture

```powershell
.\scripts\create-acceptance-fixture.ps1 -Force
```

Use the printed values in ReproDeck. The required invariant is: the original fixture remains `BAD` through Before, shadow edit, checkpoint, After and Verified Fix; it becomes `GOOD` only after explicit Apply, and ReproDeck does not create a commit.

## Windows installer

```powershell
.\scripts\release-windows.ps1
```

This runs verification and then `tauri build --bundles nsis`. The resulting installer is under `target\release\bundle\nsis` and the script prints SHA-256.

## Code rules

- Keep domain/security decisions out of React and out of Tauri command glue.
- Do not use shell string concatenation for user commands.
- Do not add automatic network calls.
- Add a regression test with every security or state-machine fix.
- Do not claim a build/test passed unless it was actually run in that environment.
