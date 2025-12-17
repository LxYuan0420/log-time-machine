# Log Time Machine (Ratatui)

Terminal log viewer focused on fast inspection: tail live logs, pause and scroll without losing lines, see activity spikes, and eventually bookmark/diff periods.

## Current controls
- Quit: `q` / `Ctrl-C`
- Pause/live: `space`, `g`/`End`
- Scroll: `Up`/`Down`/`k`/`j`, `PageUp`/`PageDown`, `Home` to top
- Timeline scrub: `Left`/`Right` to jump across bins; `s`/`S` jump to next/prev spike
- Filters: `/` to enter text (toggle regex with `R`), `F`/`C` clears, `1/2/3` toggle INFO/WARN/ERROR, `n/p` next/prev error
- Bookmarks: `b` add, `]`/`[` next/prev
- Diff: `A`/`B` set range markers; `X` clears; counts/top targets show in status; `E` exports the filtered slice
- Help overlay: `?` to toggle; timeline shows cursor/bookmark/diff markers
- Config: optional `LOGTM_CONFIG` or `~/.config/logtm/config.toml` with `max_lines = <n>`

## Run it
- Mock feed: `cargo run` or `make run-mock`
- Tail a file: `make run-sample` or `cargo run -- --file samples/sample.log`
- Tail stdin: `cat samples/sample.log | cargo run -- --stdin`
- Generate a live file: one shell `bash scripts/mock_log_stream.sh /tmp/logtm_live.log`; another `PATH="$HOME/.cargo/bin:$PATH" cargo run -- --file /tmp/logtm_live.log`

## Dev
- `make fmt` / `make check` / `make clippy`
- Code lives in `src/main.rs`; sample log at `samples/sample.log`.

## Roadmap (tracked in GitHub issues)
- Timeline cursor UI polish and bookmark marks over the timeline
- Richer diff view and export
- Test coverage and CI (fmt/clippy)
