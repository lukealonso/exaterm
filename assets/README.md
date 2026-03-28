# Logo Assets

Initial SVG logo directions for Exaterm.

Files:
- `exaterm-icon.svg`
  - rounded-square app icon
  - dark command-grid background
  - one highlighted active terminal pane

Design intent:
- avoid generic `>_` terminal branding
- emphasize a grid of terminals
- highlight one active pane to suggest supervision and attention routing

Exports:
- run `./assets/export-icons.sh` to generate common PNG sizes under `assets/generated/`

GTK wiring:
- the app currently resolves its runtime icon from `assets/icons/`
- the icon is published there under the app id `io.exaterm.Exaterm`
