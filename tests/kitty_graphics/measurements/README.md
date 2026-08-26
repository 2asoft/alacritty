# Kitty graphics maintenance measurements

`baseline.json` records the pre-maintenance working-set and scaling observations from the accepted maintenance plan. `ownership.json` records the payload-ownership milestone. `streaming.json` records the streaming-base64 milestone. `off-lock-animation.json` records the two terminal-lock sections around off-lock frame work; median lock time fell from 137,034,065 ns to 16,171 ns for insertion and from 38,584,359 ns to 242,948 ns for composition on the same host. Between the latter two runs, median peak RSS fell from 223,104 KiB to 157,528 KiB for RGBA and from 332,400 KiB to 245,460 KiB for RGB on the same host. Source inspection proves that the streaming path has no concatenated encoded body; the RSS reduction corroborates the structural result but does not independently prove allocation identity. Regenerate a comparison with:

```sh
scripts/kitty-graphics-measurements.sh target/kitty-graphics-measurements.json
```

The harness runs each case in an isolated process, records elapsed nanoseconds and Linux peak resident memory, and reports medians from a positive odd number of runs. It rejects missing cases. Results depend on the host and toolchain. Compare runs from the same host rather than treating these values as portable limits.

The cases cover ordinary text, 64 MiB direct RGB and RGBA chunk streams through the terminal parser and commit path, 4096 by 4096 frame insertion and composition while timing the terminal lock, 100,000 retained lines under three graphics states, and 65,536 virtual placements with a complete 80 by 24 placeholder viewport. Update each measurement to the replacement production boundary in the same commit that removes its current boundary.
