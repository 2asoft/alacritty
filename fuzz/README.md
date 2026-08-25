# Kitty graphics fuzzing

`kitty_stream` feeds arbitrary PTY bytes through the VTE parser, synchronized-update completion and timeout replay, ordered graphics request pipeline, decoder, terminal state, and grid resize path with a bounded graphics configuration.

Install `cargo-fuzz`, then run:

```sh
cargo fuzz run kitty_stream fuzz/corpus/kitty_stream -- -max_len=8192
```

For a bounded local smoke run without `cargo-fuzz` instrumentation:

```sh
cargo run --manifest-path fuzz/Cargo.toml --bin kitty_stream -- \
  -runs=1000 -max_len=8192 fuzz/corpus/kitty_stream
```

Use the instrumented command for security or release validation. Keep minimized reproductions in `fuzz/corpus/kitty_stream` and never commit generated artifacts.
