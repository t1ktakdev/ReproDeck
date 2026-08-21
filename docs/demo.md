# ReproDeck demo: evidence to verified fix

This is a deterministic product demo, not a mocked UI sequence. The fixture has no dependencies beyond Node.js and Git. Its expected baseline is:

- `npm run check` — PASS
- `npm test` — FAIL
- `npm run build` — PASS

The failing output exposes symptoms but does not name the defective implementation. Do not inspect the fixture generator while presenting the demo.

## Create the fixture

From the ReproDeck source directory in PowerShell:

```powershell
.\scripts\create-demo-fixture.ps1
```

The script creates `ReproDeck-Demo-Fixture` on the current user's Desktop, initializes Git, commits the baseline and validates all three commands. It refuses to overwrite an existing folder unless the caller explicitly supplies `-Force`, and fails if the intentional test unexpectedly passes. The in-app **Try Demo** path always chooses a unique folder and does not execute project commands.

## Run the product flow

1. Start ReproDeck with `npm run tauri dev`.
2. Choose **Try Demo** on Home. ReproDeck creates a real Git fixture on the Desktop and opens it without running project code. The PowerShell command above remains the reproducible CLI alternative.
3. Open **Checks**. Confirm that discovery has not executed project code.
4. Keep `check`, `test` and `build` selected. Confirm **Run selected**.
5. Verify the table reads PASS / FAILED / PASS and the failure is visible without opening full logs.
6. Choose **Start investigation**. This explicit action creates the durable Investigation Case; merely revealing the failure preview must not create one.
7. Compile context. Review every included source fragment, its reason, range and checksum. Confirm that the context budget is visible.
8. Classify evidence. Keep context neutral unless an excerpt directly supports or contradicts the current claim.
9. Optionally enable an OpenAI-compatible model and generate hypotheses. The result must contain at most three falsifiable candidates. Review citations before accepting confidence.
10. Create a Fix Workspace. Make one minimal intervention based on the best current hypothesis, then checkpoint it.
11. Review the complete diff and the proposed causal experiment before confirming execution.
12. Run the experiment. Compare baseline and experiment exit/duration, changed files, command mutation state and original integrity.
13. If the result supports the hypothesis, choose **Prepare verification session**. Review which suggested checks are required, then create the session. ReproDeck hashes and preflights the exact checkpointed patch against a separate verification workspace.
14. Run the failing **Before** first. Only after that baseline fails does Core transfer and checkpoint the candidate in the verification workspace. Run **After**, then every required regression check.
15. Review the proof chain and patch SHA-256. Apply is available only while the current patch, source commit/state, criterion and required receipts still match exactly.
16. After a successful After, make and checkpoint one harmless additional edit. Confirm Apply becomes blocked. Revert/checkpoint the intended candidate, rerun After and required checks, and confirm Ready to Apply returns.

## Safety assertions

Run these from a second PowerShell window at any point before explicit Apply:

```powershell
Set-Location "$env:USERPROFILE\Desktop\ReproDeck-Demo-Fixture"
git status --short
git rev-parse HEAD
```

The original fixture must remain clean and its HEAD must not move during Project Health, Investigation, Fix Workspace or a causal experiment. A Git worktree protects repository writes, not operating-system access; use only this trusted fixture for the public demo.

## LM Studio field-test profile

Preferred first pass: Qwen2.5 Coder 7B Instruct Q4_K_M through `http://127.0.0.1:1234/v1`. Use the exact loaded model identifier. Do not add a prompt hint that reveals the root cause.

If no model is available, complete the same flow with a manual hypothesis. The evidence, experiment and Before/After engines do not depend on AI.

## Expected records

The demo should leave:

- one Project Health run with three command receipts;
- one explicit Investigation Case;
- inspectable observed/source evidence relationships;
- one or more hypothesis artifacts;
- a causal experiment receipt;
- a separate Before/After verification session with persisted case/hypothesis/experiment links, transferred patch identity and required regression receipts.

Do not reset or delete these records before capturing screenshots.
