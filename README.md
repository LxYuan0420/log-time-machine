# Log Time Machine (Ratatui)

Fast, glanceable terminal log viewer: keep your place while new logs stream in, filter the whole line, and hop through bookmarks without losing context. Built for everyday ops/debugging rather than a toy demo.

## Features
- Tail live logs without losing lines: auto-pause when you scroll; resume with space/g.
- Whole-line filtering with live typing; toggle regex; clear in one keystroke.
- Level chips (INFO/WARN/ERROR) with strikethrough when disabled.
- Bookmark jumps with position display; timeline scrub with cursor/bookmark markers.
- Timeline bands colored by level mix for quick “what’s noisy?” reads.
- Built-in mock source so `cargo run` works out of the box; file/stdin tailing for real feeds.

## Quick start
- Watch the demo: [asciinema cast](https://asciinema.org/a/99j7kEJHxRRMmqaVo3Li2GgNk)
- Install: `cargo install ltm`
- Default mock (no setup): `cargo run`
- Tail a file: `cargo run -- --file <path-to-your-log>` (e.g., `samples/sample.log`)
- Tail stdin: `cat <your-log> | cargo run -- --stdin`
- Generate and view a live file (script produces the stream, TUI tails it): `bash scripts/mock_log_stream.sh /tmp/logtm_live.log >/dev/null 2>&1 & cargo run -- --file /tmp/logtm_live.log`

<a href="https://asciinema.org/a/99j7kEJHxRRMmqaVo3Li2GgNk" target="_blank">
  <img src="https://asciinema.org/a/99j7kEJHxRRMmqaVo3Li2GgNk.svg" alt="asciicast" width="400"/>
</a>

## Controls (in-app command bar)
- Quit: `q` / `Ctrl-C`
- Pause/live: `space`, `g`/`End`
- Scroll: `Up`/`Down`/`k`/`j`, `PageUp`/`PageDown`, `Home`/`End`
- Timeline: `Left`/`Right`
- Filters: `/` to type (filter matches timestamp/level/target/message), `Enter` apply, `Esc` cancel, `F/C` clear, `R` regex, `1/2/3` toggle INFO/WARN/ERROR, `n/p` next/prev error
- Bookmarks: `b` add, `]`/`[` next/prev (status shows which bookmark you’re on)
- Help: `?`

## Configuration
Optional `LOGTM_CONFIG` or `~/.config/logtm/config.toml` with `max_lines = <n>` to cap retained lines. Defaults keep memory bounded.
