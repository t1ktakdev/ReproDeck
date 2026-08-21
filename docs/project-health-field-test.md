# Project Health manual field test (Windows)

Use only the generated fixture or another repository you trust. Project Health is worktree-isolated but not an OS sandbox.

```powershell
cd "$HOME\Desktop\ReproDeck-Next"
.\scripts\create-project-health-fixture.ps1 -Force
```

Open `C:\Users\<you>\Desktop\ReproDeck-Health-Fixture` in ReproDeck.

1. Open **Checks**. The declared `npm test` check should be detected and selected.
2. Click **Run selected** and accept the execution warning.
3. Expected result: `Failed`, exit code `1`, redacted recorded stderr, and a `health:*` evidence ID.
4. Open **Problems**. One active **Reproduced failure** should exist. Static discovery signals remain separate.
5. In PowerShell confirm the source repository stayed untouched by Project Health:

   ```powershell
   cd "$HOME\Desktop\ReproDeck-Health-Fixture"
   Get-Content .\state.txt
   git status --short
   git log --oneline -1
   ```

   Expected: `BAD`, clean status, original commit unchanged.
6. Change `state.txt` to `GOOD` in the original fixture and commit it yourself. Re-scan the project in ReproDeck.
7. Run the same health check again. Expected result: `Passed`.
8. Open **Problems**. The old failure remains in history as **Not reproducing now**. It must **not** be labeled `Verified Fix`, because this field test did not prove which change caused the pass.

This test validates the Project Health lifecycle independently from the older Before/After Apply proof workflow.
