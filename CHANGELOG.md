# Changelog

## 1.0.0

A complete redesign around one idea: open a log and immediately see something worth reading.

### Chart

- New chart engine: stacked panes grouped by unit, a shared time axis, live values for every series under the cursor, a crosshair across panes and an overview strip of the whole recording.
- Smooth with millions of samples: per-pixel min/max decimation keeps every spike visible while drawing only what the screen can show.
- Drag to select a range, wheel to zoom, right-drag to pan, double-click to fit, <kbd>Alt</kbd>+arrows to step back and forward through views. Pane actions for log scale, fit Y, include zero, lock Y, maximize and remove.
- Time axes read like clocks and dates; elapsed seconds read as `m:ss`, sub-second captures in ms, µs or ns; numbers use thousands separators and SI prefixes where they help. Log axes label 2× and 5× steps across a few decades.
- Flags such as thermal throttling are drawn as steps labelled No/Yes.
- Panes with many series keep a compact legend; hovering the pane lists every value.

### Finding and reading data

- Smart first view: the time axis and the channels an engineer usually looks first are selected automatically (package power, effective clock, package temperature for CPU logs).
- Channel browser grouped by unit with fuzzy search, sparklines, units and multi-source availability badges.
- Statistics dock for the selection, the visible range or everything: min, mean, max, std, percentiles, RMS, slope, time above threshold, energy in J/kJ/Wh and more; sortable and copyable for Excel.
- Comparing runs: separate recordings opened together are aligned at their start automatically (with Undo) and labelled by what differs in their names (`eco`, `performance`); offset them in time and compare their statistics against a reference run with Δ and Δ% per statistic.
- Logs that interleave several devices row by row (nvidia-smi, Intel PTAT, turbostat) are split into one source per device automatically, named after the device (with Undo).
- Source operations: merge sources on a common X, stack rows, transpose, split by any column; generated sources are rebuilt when a session is opened.

### Import

- Much more robust delimited-text import: encoding detection (UTF-8, UTF-16, Windows-1252), delimiter and decimal-comma detection, metadata preambles, units rows, LibreHardwareMonitor headers, repeated header rows, unit-suffixed values, Yes/No flags, merged Date + Time columns and inferred day/month order.
- New reader for MSI Afterburner `.hml` logs. Telemetry `.txt`/`.log` files are no longer mistaken for simulation formats.
- Columns such as `charge [C]` are read as coulombs (A·s) instead of degrees Celsius.
- Loading happens in the background with progress; problems are listed instead of interrupting.

### Tools

- New channel dialog with templates (sum, difference, ratio, efficiency, rate of change, running energy, moving average) and a live preview; new formula functions `cumtrapz` and `movmean`.
- Processing editor with typed forms for every step, saved and loaded as JSON.
- Analyze window: curve fitting with overlays, damping estimation, fatigue equivalent loads and a statistics report, all limited to the selected range when there is one.
- Inspector (<kbd>Ctrl</kbd>+<kbd>I</kbd>) for series colours, widths, dashes and panes; marks (points, regions, events, reference lines) edited in the axis' own time format; and filters that hide samples outside chosen ranges without changing the data.
- 2D field view with colormaps, polar projection and export; files that only contain fields open in it directly.

### Sharing and workflow

- Figure export with live preview: PNG, SVG (text as outlines), PDF and EPS, light or dark, any size and resolution; copy the figure to the clipboard in one keystroke.
- Data export of the plotted channels or all columns, for everything, the visible range or the selection.
- Python script export that reproduces the chart.
- Sessions (`.dvsession`) save files, channels, panes, styles, marks, processing and the view.
- Command palette (<kbd>Ctrl</kbd>+<kbd>K</kbd>), keyboard shortcuts for everything common, follow mode for files that are still being written.
- New look: Inter typography with tabular figures, dark and light themes that follow Windows, and the new rustDatView logo and icon.

### Removed

- The settings preset and draft-recovery system. Preferences are now ordinary settings, and everything about a piece of work is saved explicitly as a session.
- macOS and Linux release archives. Releases are built for Windows; the source still builds elsewhere.

## 0.1.1

- Native Windows and macOS release binaries.
- Start from default settings; restoring previous settings is explicit.

## 0.1.0

- First Rust implementation of pyDatView.
