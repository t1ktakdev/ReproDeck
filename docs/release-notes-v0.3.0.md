# ReproDeck v0.3.0 — Evidence-first debugging

ReproDeck v0.3.0 is the first public release of the evidence-first desktop debugging workbench for AI-assisted code fixes.

## Highlights

- Evidence-first Investigation Cases with inspectable relationships
- Structured, falsifiable hypotheses and causal experiments
- Isolated Fix Workspaces that keep the original repository unchanged
- Exact verified patch identity with Before/After proof
- Required regression gates for the same patch bytes
- Explicit, backend-protected Apply
- Optional local AI through LM Studio or another OpenAI-compatible provider
- Keyboard-first Workbench UI, recovery records, and replayable capsules

## Download

Download the Windows x64 per-user installer from the assets below:

- `ReproDeck_0.3.0_x64-setup.exe`
- `SHA256SUMS.txt`

Windows 11 is tested. The community installer is not currently Authenticode-signed, so Windows may show a publisher warning. Verify the downloaded file against `SHA256SUMS.txt`.

## Safety note

Git worktrees isolate repository writes but are not an operating-system sandbox. Project code runs with the current user's permissions. Run only code you trust. AI output is a proposal; evidence and exact-patch verification remain authoritative.

## Known limitations

- The Windows installer is unsigned.
- Linux desktop runtime field testing is pending; macOS is not runtime-tested.
- Large-repository performance validation and broader controlled benchmark coverage remain roadmap work.
- No portable Windows build is claimed for v0.3.0.

See the [full changelog](https://github.com/t1ktakdev/ReproDeck/blob/v0.3.0/CHANGELOG.md), [security model](https://github.com/t1ktakdev/ReproDeck/blob/v0.3.0/SECURITY.md), and [demo walkthrough](https://github.com/t1ktakdev/ReproDeck/blob/v0.3.0/docs/demo.md).

## Русский

ReproDeck v0.3.0 — первый публичный релиз evidence-first workbench для отладки исправлений, предложенных AI. Он связывает реальные падения, evidence, каузальные эксперименты и Before/After-проверку точного patch до явного Apply. Windows 11 протестирован; installer пока не подписан. Подробнее: [README на русском](https://github.com/t1ktakdev/ReproDeck/blob/v0.3.0/README.ru.md).
