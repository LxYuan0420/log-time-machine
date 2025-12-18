# Log Time Machine (Ratatui)

Terminal log viewer focused on fast inspection: tail live logs, pause and scroll without losing lines, see activity spikes, and jump across bookmarks.

## Current controls
- Quit: `q` / `Ctrl-C`
- Pause/live: `space`, `g`/`End`
- Scroll: `Up`/`Down`/`k`/`j`, `PageUp`/`PageDown`, `Home` to top
- Timeline scrub: `Left`/`Right` to jump across bins
- Filters: `/` to enter text (toggle regex with `R`), `F`/`C` clears, `1/2/3` toggle INFO/WARN/ERROR, `n/p` next/prev error. Filters match the whole row (timestamp, level, target, message) and show what you type.
- Bookmarks: `b` add, `]`/`[` next/prev, status shows which bookmark you’re on
- Timeline colors: red=errors, yellow=warnings, white=info. Markers: `^` cursor, `*` bookmark, `#` cursor+bookmark.
- Help overlay: `?` to toggle; timeline shows cursor/bookmark markers
- Config: optional `LOGTM_CONFIG` or `~/.config/logtm/config.toml` with `max_lines = <n>`

## Run it
- Mock feed (default if no source provided): `cargo run` or `make run-mock`
- Tail a file: `make run-sample` or `cargo run -- --file samples/sample.log`
- Tail stdin: `cat samples/sample.log | cargo run -- --stdin`
- Generate a live file: one shell `bash scripts/mock_log_stream.sh /tmp/logtm_live.log`; another `cargo run -- --file /tmp/logtm_live.log` (or one-liner: `bash scripts/mock_log_stream.sh /tmp/logtm_live.log >/dev/null 2>&1 & cargo run -- --file /tmp/logtm_live.log`)
- Replay the demo: `asciinema play docs/demo.cast` (cast recorded from the built-in mock run)

## Dev
- `make fmt` / `make check` / `make clippy`
- Code lives in `src/main.rs`; sample log at `samples/sample.log`.

## Roadmap (tracked in GitHub issues)
- Timeline cursor UI polish and bookmark markers
- Filter polish and saved presets
- Test coverage and CI (fmt/clippy)
