# pixel-cast: instructions for Claude

Rust command-line tool that streams test patterns, images, GIFs and videos to LED matrices over E1.31 (sACN). The
plan, decisions and steps are in `PLAN.md`: it is the authority. Tick its step boxes and update its **Status** line
as work lands; record any new decision there (with its date), not only in the conversation.

## Language

- **English everywhere**: code, identifiers, comments, messages, file names, documentation (`README.md`, `PLAN.md`,
  this file), commit messages. The repository is public.
- Conversations with the user stay in French.

## Standalone

- pixel-cast is a standalone, generic tool: it knows E1.31 and matrix geometry only, nothing about any particular
  receiver, display or project that uses it.
- **Tracked files never mention other projects, private hosts or addresses, or local paths**: not in code, tests,
  docs, examples or commit messages. Examples use neutral names (`wall`) and documentation addresses (`192.0.2.x`,
  RFC 5737).
- Machine-specific context (reference implementations, test receivers, projects that use pixel-cast) lives in
  `CLAUDE.local.md`, which is git-ignored. Use it, but never copy its content into tracked files.
- The first version is the command line of `PLAN.md`; anything listed under "After the first version" waits for an
  explicit request.

## Code

- Module layout and dependency choices: `PLAN.md`, "Architecture". Ask before adding a dependency that is not listed
  there.
- The binary must stay self-contained and statically linkable (musl): no linked C library. Video goes through
  `ffmpeg` as a subprocess, only for the `video` source.
- E1.31 packets are hand-written (ANSI E1.31-2025). Keep packet building pure (no I/O) so it can be tested byte for byte.
- The standard's PDF, when the user has downloaded it, is in `docs/standard/` (git-ignored: it is copyrighted). Check
  packet layout and section numbers against it; never commit it or quote long passages of it.
- Output contract for other programs, to keep stable once released:
  - stdout carries only machine output (`--json` lines, `--dry-run` report); human messages go to stderr;
  - exit codes: 0 normal end, 2 usage, 3 unreadable file, 4 network, 5 ffmpeg missing or failed;
  - Ctrl+C and SIGTERM end the stream cleanly (stream-terminated packets, E1.31 §6.2.6).

## Checks

Before saying a change is done, run:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Tests never send to the real network: use loopback (a local UDP receiver that rebuilds the picture) or `--dry-run`.

## Real receivers and publishing

- **Never stream to a real receiver** on the network without the user asking: a software receiver comes first
  (step 5 of `PLAN.md`).
- Creating the GitHub repository, pushing, tagging a release or publishing to crates.io or a container registry:
  only with the user's explicit go-ahead, each time.
