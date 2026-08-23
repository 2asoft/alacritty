# Current observation

This bundle records behavior. Differences do not fail the suite.

| Terminal | Available | KGP query OK | Response sequence | Red classic pixels | Green virtual pixels | Leaked magenta pixels |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| alacritty | yes | yes | kgp-ok, text-area-pixels, screen-size-cells, primary-device-attributes | 1296 | 162 | 0 |
| kitty | yes | yes | kgp-ok, text-area-pixels, cell-size-pixels, screen-size-cells, primary-device-attributes | 1156 | 162 | 0 |

## Completion expectation

Alacritty should build, answer the KGP query, render both fixtures, replace the magenta placeholder glyph with the virtual image, and preserve the query response order recorded in `report.json`.

The full-frame Alacritty/Kitty comparison contains 10775 differing pixels.
