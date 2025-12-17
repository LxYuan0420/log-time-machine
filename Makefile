fmt:
	cargo fmt

clippy:
	cargo clippy -- -D warnings

check:
	cargo check

run-sample:
	cargo run -- --file samples/sample.log

run-mock:
	cargo run
