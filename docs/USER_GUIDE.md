# rustDatView user guide

This is the reference for everything rustDatView 1.0 can do. For a short tour of the features you are least likely to find elsewhere, start with the [quick start](QUICK_START.md).

- [Starting](#starting)
- [Window layout](#window-layout)
- [Opening files](#opening-files)
- [Sources](#sources)
- [Channels](#channels)
- [The chart](#the-chart)
- [Modes](#modes)
- [Statistics](#statistics)
- [Inspector: series, marks and filters](#inspector-series-marks-and-filters)
- [New channels and formulas](#new-channels-and-formulas)
- [Processing pipeline](#processing-pipeline)
- [Analyze tools](#analyze-tools)
- [Table, Info and Fields views](#table-info-and-fields-views)
- [Exporting](#exporting)
- [Sessions](#sessions)
- [Following live files](#following-live-files)
- [Command palette](#command-palette)
- [Settings](#settings)
- [Command line](#command-line)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Limitations](#limitations)

## Starting

Extract the release archive and run `rustDatView.exe`. Nothing needs to be installed. The archive also contains an `examples` folder. On Windows 11 24H2 and later, no console window opens alongside the app. On older Windows, one appears briefly and closes.

Without files, the window shows a welcome screen:

- **Open file…** opens a file picker.
- **Open sample data** opens one of four built-in examples: *CPU gaming session*, *Compare two benchmark runs*, *Signals & spectrum* and *Motherboard power*. They are written to `%TEMP%\rustDatView-samples` first.
- **RECENT FILES** and **RECENT SESSIONS** list up to 12 items each. Items whose file no longer exists can be removed with the button beside them.

You can also drop files anywhere on the window, or pass them on the [command line](#command-line).

## Window layout

| Area | What it holds |
| --- | --- |
| Top bar | Menus (File, Edit, View, Data, Analyze, Help), the mode switcher (Chart view only), the view switcher (Chart, Table, Info and, when available, Fields) and the command palette button |
| Sidebar (Ctrl+B) | **Sources**: the open files and generated sources. **Channels**: the X axis, channel search and the channel list |
| Chart | The plot, its toolbar and, for some modes, a mode options bar |
| Statistics (Ctrl+J) | A table of statistics for every plotted series. Shown in Chart view |
| Inspector (Ctrl+I) | Series styles, marks and filters. Hidden by default. Shown in Chart view |
| Status bar | The first source's name, rows, channels, sample rate and duration; loading progress; the problems button; the **Follow** button; the X value under the cursor |

The window title shows the first source's name, followed by `+ N more` when several sources are open.

Notifications appear at the bottom right, at most three at a time. Ordinary ones disappear after 4 seconds, ones with an **Undo** button after 10 seconds. Errors stay until dismissed. Every error is also kept in the problems list: click the warning button in the status bar to read, **Copy** or **Clear** them.

## Opening files

| How | Effect |
| --- | --- |
| File → Open file… (Ctrl+O), welcome screen, plain drag and drop | Replaces everything that is open |
| File → Add files… (Ctrl+Shift+O), the **+** button above the sources, drop while holding Ctrl or Shift | Adds to what is open. A file that is already open is not opened twice |
| File → Open with format… | Choose the reader yourself, then pick the files. **Auto detect** is the default |
| File → Reload (F5) | Reads every file again. Right-click a source → **Reload source** reloads one file |
| Command line | `rustDatView.exe file1.csv file2.csv` |

Files load in the background. The status bar shows a spinner, `Loading <file>` and the elapsed time. If a reload fails, the previously loaded data for that file is kept. Files with several tables (for example Excel sheets or several MSI Afterburner sessions) open as one source per table.

### File formats

The reader is chosen from the file's extension and content. When several readers claim an extension, they are tried in order until one accepts the file. **Open with format…** and `--format` force a reader.

| Reader | Extensions |
| --- | --- |
| CSV file | .csv, .txt, .log |
| Excel file | .xls, .xlsx |
| MSI Afterburner HML | .hml |
| Parquet file | .parquet |
| NetCDF file (<=2D) | .nc |
| TDMS file | .tdms |
| VTK file | .vtk, .vtp, .vti, .vtr, .vts, .vtu |
| Matlab mat file | .mat |
| Pickle file | .pkl (tables are decoded; no Python code is run) |
| Tecplot ASCII file | .dat |
| GNUPlot file | .dat, .raw |
| Bladed output file | .%\*, .$\* |
| FAST input file | .dat, .fst, .txt, .fstf, .dvr |
| FAST output file | .out, .outb, .elm, .elev, .dbg, .dbg2 |
| FAST linearization output, FAST summary file, FAST determ. wind file | .lin; .sum, .yaml; .wnd |
| TurbSim time series, TurbSim binary | .txt; .bts |
| HAWC2 dat, htc, st, PC and AE files; HAWC2 Turbulence box | .dat, .sel, .htc, .st, .pc, .ae; .u, .v, .w, .bin |
| HAWCStab2 power, induction and Campbell files | .pwr, .ind, .cmb, .txt |
| FLEX blade, profile, output, WaveKin and documentation files | .bld, .bla, .pro, .res, .int, .wko, .out, .doc, .00x |
| BModes output file | .out |
| ROSCO DISCON file, ROSCO Performance file | .in; .txt |
| CACTUS file, RAAW .mat file | .in; .mat |

`rustDatView.exe --help` prints the exact reader names accepted by `--format`.

### Delimited text

Text logs are read according to their content, not their extension:

- **Encoding**: UTF-8 with or without BOM, UTF-16 with a BOM, otherwise Windows-1252.
- **Delimiter**: comma, semicolon, tab, pipe or whitespace, detected from the rows. With a semicolon delimiter, a decimal comma is accepted (`0,573`). `'` and spaces are accepted as thousands separators, and `0x…` values as hexadecimal.
- **Header**: preamble lines before the table are skipped (oscilloscope and power-analyzer exports, PTAT wrappers). A row of units under the header is used as units, including units the app does not otherwise know (`s, bar, deg, L/min, W`). LibreHardwareMonitor's two header rows are combined. Repeated header rows from concatenated logs are skipped. HWiNFO's trailing *Sensor Group* row is skipped.
- **Units** in brackets or parentheses in the header (`Power [W]`, `Time (s)`) are recognised, and so are logger-style suffixes after an underscore (`Power_W`, `Time_s`, `Clock_MHz`, `Load_pct`). Suffixes that usually belong to the name are left alone (`Phase_A`, `Temp_min`). Values with a unit suffix (`58.46 W`, `640 MHz`, `14 %`) are split into number and unit. A column that mixes different units is kept as text.
- **Missing values**: empty fields, `-`, `NA`, `N/A`, `[N/A]`, `NaN`, `NaT`, `NULL`, `None` and `<NA>`.
- **Flags**: `Yes`/`No`, `True`/`False` and `On`/`Off` become 1/0 with the unit `Yes/No`. They are drawn as steps in a *Flags* pane.
- **Dates and times**: separate `Date` and `Time` columns are merged into one `Time` timestamp. Whether `03/04` is 3 April or 4 March is inferred from the data; when it cannot be, month first is used unless **Day first when ambiguous** is on in Settings (or `--dayfirst` is given). A column named for time (`timestamp`, `ts_ms`, `Time_Of_Day_Seconds`…) holding Unix time in seconds, milliseconds, microseconds or nanoseconds is read as dates (2000–2100); a unit such as `_ms` drops from the name. A Rigol-style `X` sequence column with *Start* and *Increment* produces a `Time [s]` column.
- **Ragged rows**: short rows are padded with missing values; extra fields go into new columns `C1`, `C2`…; an empty trailing column is dropped.
- **Duplicate names** get ` #2`, ` #3`, … appended.
- **Known producers**: units are filled in for LibreHardwareMonitor sensor IDs, Intel PTAT, turbostat, PresentMon, Joulescope statistics and nvidia-smi columns that do not carry them.

Everything the reader decided (encoding, separator, skipped lines, merged columns, converted flags) is listed under *Reader notes* in the [Info view](#info-view).

### MSI Afterburner HML

`.hml` files are read record by record: channel names, units and samples. A log with several recording sessions opens as one source per session (`Session 1`, `Session 2`, …). Unnamed channels become `Channel N`.

### Units

Units decide pane grouping, statistics units, energy and comparisons. rustDatView recognises SI prefixes and the families power (W), voltage (V), current (A), frequency (Hz), temperature (°C, K, °F, also written `degC`, `C`, `celsius`…), percent, fan speed (rpm), data (B, with SI or IEC prefixes such as MiB), throughput (B/s), time (s, min, h, d), energy (J, Wh), charge (A·s, Ah), frame rate (fps) and flags (Yes/No). A recognised unit is shown in one spelling everywhere, whatever the file wrote: `degC` and `C` appear as °C, `RPM` as rpm. Other units are kept as written.

## Sources

Each open file (or table within a file) is a *source*. The source card shows the name, the row and channel count and, for time-based data, the duration and sample rate.

- **Checkbox**: includes the source in the chart. Checked sources are also the ones that **⋯** actions operate on.
- **Name**: hover for the full path. Right-click → **Rename** renames it (Enter to confirm, Escape to cancel).
- **Colour dot**: shown when series are coloured by source.

### Source menu (right-click a source name)

| Item | Effect |
| --- | --- |
| Rename | Rename the source |
| Reload source | Read this file again |
| Show in table | Open the Table view on this source |
| File information | Open the Info view |
| Export data… | Open the data export dialog for this source |
| Align start to 0 | Subtract the first X value, so the source starts at zero |
| X offset | Shift the source along X |
| Split by… | Make one source per value of a column. Lists columns with 2–64 distinct values (`name · N values`) |
| Transpose | Add a transposed copy as a new source (`<name> · transposed`) |
| Move up, Move down | Reorder sources. This is the only way to reorder them |
| Remove from workspace | Close the source |

### Offsets and alignment

When more than one source is checked, each checked source shows **Start at 0** and an **Offset** field (in seconds for time axes, otherwise in the X unit). Use them to line up runs by hand.

**Separate recordings are aligned automatically.** When two or more checked sources cover time ranges that do not overlap (for example the same benchmark run on different days), they are all set to start at 0 and the notification `N separate recordings aligned at their start` offers **Undo**. Recordings that overlap in time keep their absolute timestamps, so simultaneous loggers stay in sync.

### Interleaved devices

Some tools write several devices into one file, one row per device per timestamp: nvidia-smi with several GPUs, Intel PTAT and turbostat with package and per-core rows. When a file is opened, rustDatView looks for repeating timestamps and a column that tells the rows apart (such as `index` or `name`). If it finds one, it creates one source per device, unchecks the original and shows `<file> interleaves N devices; split by <column>` with **Undo**. After Undo, the original source's card offers the link `Rows interleave N devices — Split by <column>` to split it later.

You can split any source yourself with **Split by…** in its menu.

### Source actions (the ⋯ button above the sources)

| Item | Effect |
| --- | --- |
| Sort by name | Sort sources alphabetically |
| Align all starts | Start every checked source at zero and clear offsets |
| Merge selected on X | Join the checked sources into one source on the current X axis. Every channel is interpolated onto the shared X values and left empty outside its own range. Columns are named `<channel> · <source>` |
| Stack selected rows | Append the rows of the checked sources into one source |
| Stack common columns only | When stacking, keep only columns present in every source |
| Transpose selected | Transpose every checked source |

Merged, stacked, split and transposed sources are *generated* sources. Their parents are unchecked, not removed. Generated sources are named after what differs between the parents, for example `eco + performance (merged)`. Sessions store how they were made and rebuild them from the parent files.

Merge needs an X axis that is numeric (or a timestamp), finite and sorted in every source, with compatible units. The errors say which source is the problem.

## Channels

### X axis

The **X** box selects the X axis. rustDatView picks a time column by default (a column called *Time*, *Elapsed*, *Timestamp*, *Date/Time*, *TimeInSeconds* and similar, that is a timestamp or increasing in a time unit). Otherwise it uses an index column or the first numeric column. Right-click a channel → **Set as X axis** also sets it.

### Initial channels

When a file is opened, a first set of channels is plotted:

- Files with at most 8 numeric columns: all of them, except ID and counter columns.
- Hardware telemetry (power, clock, temperature and load channels): one power channel (package power first), one clock (effective clock average first) and one temperature (CPU package first).
- Otherwise up to three varying channels of different kinds, preferring power, clock, temperature and frame time.

### Finding channels

Type in **Find a channel…** (Ctrl+F or /). Every word must occur somewhere in the name, in any order. A word also matches when its letters appear in order with others in between, so `pkg pwr` finds *CPU Package Power*. Closer matches are listed first. Use `|` for alternatives: `vrm | soc`. Matching ignores case.

- Enter plots or hides the first match. Ctrl+Enter selects every match.
- Escape clears the search.
- ↑/↓ move through the list; Space or Enter toggles the focused channel.
- **⋯ → Select matching channels** selects every match; **Clear selection** clears the selection.

The counter shows `N of M selected`.

### The channel list

Channels are grouped by unit family (POWER, CLOCK, TEMPERATURE…). The **⋯** menu beside **Channels** switches between **Group by unit** and **Source order**, and can **Show index channel**.

Each row shows a dot (filled in the series colour when plotted), the name, the unit and a sparkline. Hover a row for its full name, unit and min/mean/max. When several sources are checked and a channel exists in only some of them, a badge such as `1/2` shows in how many.

| Click | Effect |
| --- | --- |
| Click | Plot or hide the channel |
| Shift+click | Add every channel from the last clicked one to this one |
| Alt+click | Plot only this channel |

Right-click a channel for **Set as X axis**, **Plot only this channel**, **New channel from this** (formula templates), **Edit formula…** (formula channels only) and **Copy channel name**.

The **{ }** button beside **Channels** opens [New channel](#new-channels-and-formulas) (Ctrl+N).

A channel plotted with several sources checked appears once per source, labelled with the part of the source name that differs, for example `CPU Package Power · eco` and `CPU Package Power · performance`.

## The chart

### Toolbar

| Control | Effect |
| --- | --- |
| Select / Pan / Zoom | What a left-drag does. Select is the default |
| Fit button | Fit all data (Home, 0 or double-click) |
| Log Y | Logarithmic Y on every pane |
| Panes | Pane grouping and series colours |
| Display | Grid, legend, crosshair and axis options |
| Zoom to selection, clear selection | Shown while a range is selected |
| Inspector button | Show or hide the inspector |

### Mouse and keyboard

| Action | How |
| --- | --- |
| Select a range | Drag (Select tool). Drag the edges to resize, drag inside to move. Click outside to clear |
| Box zoom | Ctrl+drag, or drag with the Zoom tool |
| Pan X | Right- or middle-drag with any tool, or drag with the Pan tool. Shift+wheel, ←/→ (10%) |
| Pan Y too | Hold Shift while panning, or start the pan on the Y axis |
| Zoom X | Wheel or pinch; +/− on the keyboard |
| Zoom Y of one pane | Ctrl+wheel, or wheel over the Y axis |
| Zoom to the selection | Double-click inside it, or **Zoom to selection** |
| Fit Y of one pane | Double-click its Y axis, or its **Fit** button |
| Fit everything | Double-click elsewhere, Home or 0 |
| Equalise two pane heights | Double-click the divider between them. Drag a divider to resize panes |
| Back / forward through views | Alt+← / Alt+→ (up to 100 steps) |
| Clear the selection | Escape. This also switches back to the Select tool |
| Hide or show a series | Click its legend entry |
| Show only one series | Alt+click its legend entry. Alt+click it again to restore |

Keyboard keys work while the pointer is over the chart and no text field has focus.

### Panes

Series are grouped into stacked panes that share the X axis. **Panes** chooses the grouping:

- **By unit** (default): one pane per quantity, titled for example `POWER · W`, `CLOCK · MHz`, `TEMPERATURE · °C`. Yes/No flags go to a `Flags` pane drawn as steps.
- **Single pane**: everything in one pane.
- **One per channel**: one pane per channel.
- **Custom**: type a pane name for each channel. *Channels with the same pane name share an axis.* Moving a series to another pane in the inspector also switches to Custom.

**Color series by**: **Auto** colours by source when several sources are shown and a pane has a single channel, and by channel otherwise. **Channel** and **Source** force one or the other. Unless colouring by source, the second, third and fourth sources are drawn dashed, dotted and dash-dotted.

Hover a pane (at least 320 × 60 pixels) to show its buttons:

| Button | Effect |
| --- | --- |
| Log | Logarithmic Y for this pane |
| Fit | Fit Y to the visible data |
| Zero | Always include zero on Y |
| Lock | Keep this Y range while panning and zooming |
| Max | Maximize or restore this pane |
| × | Remove this pane's channels from the chart |

### Display menu

| Option | Default |
| --- | --- |
| Grid | On |
| Legend | On |
| Values in legend while hovering | On |
| Crosshair | On |
| Tooltip with every value | Off |
| Link X across panes | On. When off, each pane zooms and pans X on its own |
| Include zero on Y | Off |
| Logarithmic X | Off |
| Reverse X, Reverse Y | Off |
| Equal X/Y scale | Off (always on in Polar mode) |

### Reading values

While hovering, the crosshair marks the samples under the cursor, the legend shows each series' value, and a pill on the X axis shows the exact X (full date and time with milliseconds for timestamps). The status bar repeats it as `<X channel> = <value>`. When a pane has more series than fit, the legend ends in `+N more`; hover it to see them all.

Lines are broken across missing samples and across gaps longer than 10 times the usual sample interval (configurable in Settings). Dense data is reduced to the minimum and maximum per pixel column, so peaks are never lost; statistics always use every sample.

### Overview strip

For time series with at least 2,000 samples, a strip below the chart shows the whole recording with the visible range highlighted. Drag the highlighted window or its edges to move or resize the view. It can be turned off in Settings.

### Right-click menu

Right-click a series for its name, then:

- **Hide series** / **Show series**, **Show only this series**, **Remove series**.
- **Style**: **Color**, **Width**, **Dash** (Solid, Dashed, Dotted, Dash-dot), **Draw** (Lines, Points, Lines + points, Steps) and **Reset style**.
- **Add point annotation here**, **Add region from selection**, **Add reference line at this value**, **Add event marker here**. See [Marks](#marks).
- **Zoom to selection**, **Fit all**, **Fit Y in every pane**.
- **Copy X and Y**: the position under the cursor as tab-separated text.
- **Copy figure**, **Export figure…**.

## Modes

The mode switcher in the top bar (Alt+1 to Alt+6) changes how the selected channels are drawn. Changing mode resets zoom and selection. Every mode except Signals has an options bar above the chart.

### Signals (Alt+1)

Channels over the X axis. This is the default.

### Distribution (Alt+2)

The probability density of each channel's values. **Bins** sets the number of histogram bins (default 30). **Smooth (KDE)** draws a kernel density estimate instead. The Y unit is the inverse of the channel's unit.

### Spectrum (Alt+3)

The frequency content of each channel. Log Y is switched on when you enter this mode.

| Option | Choices |
| --- | --- |
| Output | Amplitude, PSD (default), f × PSD |
| X axis | Frequency (default), Period, Angular frequency |
| Averaging | None, Welch (default), Binning |
| Window (Welch) | Hamming (default), Hann, Rectangular |
| Segment power (Welch) | Segment length is 2 to this power; default 11 (2,048 samples) |
| Points per decade (Binning) | Logarithmic bins per factor of ten; default 20 |
| Detrend | Subtract the mean first (per segment for Welch) |

With X in seconds or timestamps, frequency is in Hz and PSD in unit²/Hz.

### Normalized (Alt+4)

**Scale Y** maps each channel to 0–1 (on by default). **Scale X** maps X to 0–1. **Centering** shifts each channel instead: *Midpoint at zero*, *Midpoint at reference*, *Mean at zero* or *Mean at reference*, with **Reference** giving the value. Scale Y applies only when Centering is None.

### Compare (Alt+5)

Each channel is compared with the first one in its group (the reference). Compared channels are interpolated onto the reference's X.

- **Method**: *Relative %* (default), *Absolute relative %*, *Ratio*, *Difference*, or *Signal vs reference* (the compared channel plotted against the reference).
- **Pairing**: how channels are grouped. **Auto** compares channels of one source when one source is shown; the same channel across sources when one channel is selected; and channels with similar names across sources otherwise. You can also choose *Tables, one channel*, *Channels of one table*, *Tables, same channels* or *Tables, similar channels*.

### Polar (Alt+6)

The X channel is the angle and each Y channel the radius. **Degrees** (on by default) reads angles in degrees. **Bins** averages values in 12, 36, 60, 180 or 360 angular bins. **Axis** chooses whether the angle is measured about the X or Z axis. **Radius reference** shifts the mean radius to a chosen value. The X channel must be numeric.

## Statistics

The statistics dock (Ctrl+J) shows one row per plotted series, computed from the original samples.

**Scope**:

- **Selection**: samples inside the selected range. Only offered while a range is selected; selecting a range switches to it.
- **View**: samples in the visible range. When X is not linked across panes, each series uses its own pane's range.
- **All**: every sample.

With a selection, its start, end and length are shown next to the scope.

### Columns

Choose columns with the sliders button. The defaults are Min, Mean, Max, Std, P99, Energy and N. Columns appear in the order you turn them on.

| Column | Meaning |
| --- | --- |
| N | Number of valid samples |
| Min, Mean, Max, Median | |
| Std, Variance | Population standard deviation and variance |
| RMS | Root mean square |
| P1, P5, P95, P99 | Percentiles, with linear interpolation |
| Energy | Integral of a power channel over time, shown in J, kJ, Wh, MJ or kWh |
| ∫ dx | Trapezoidal integral over X (unit × X unit) |
| Integral mean | ∫ dx divided by the covered X span |
| Peak-to-peak | Max − Min |
| Abs max | Largest absolute value |
| Start, End, Δ | First value, last value and End − Start |
| Slope | Least-squares slope |
| Time above threshold | How long the series is above **Threshold** (in each series' own unit), interpolating between samples |
| X at min, X at max | Where the minimum and maximum occur |
| Duration | X span covered by samples |
| Mean Δx | Mean sample spacing |
| TI | Std / Mean |
| Integral x, Integral x normalized, Integral x² | ∫ y·x dx, its ratio to ∫ y dx, and ∫ y·x² dx |

**Energy** needs a power unit (W, mW, kW…), a time X axis and Signals mode; otherwise it shows `—`. So does any value that cannot be computed.

Click a column header to sort; click again to reverse.

### Compare to

When more than one source is checked, **Compare to** adds a `Δ` column after each statistic: the difference from the same channel in the reference source, and in brackets that difference relative to the reference. Choose the reference source in the box next to it. Values are converted to the row's unit before subtracting.

### Copying

The copy button offers **Copy for Excel (TSV)** (also Ctrl+Shift+C after clicking in the dock), **Copy as CSV**, **Copy as Markdown** and **Export as CSV…**. Copies contain full-precision numbers, a unit column per statistic where needed, and Δ, Δ unit and Δ% columns when comparing. Energy is exported in joules, timestamps in seconds since the Unix epoch (UTC).

Statistics for very long series are computed in the background; *Updating statistics…* shows meanwhile.

## Inspector: series, marks and filters

Open the inspector with Ctrl+I or the toolbar button. It has three tabs.

### Series

One card per plotted series: hide or show it, see its unit and source, or remove it from the chart. **Style & pane** sets **Color**, **Width**, **Dash** and **Draw** (Lines, Points, Lines + points, Steps), moves the series to another pane, or to a new pane (type a name and press **Move**). **Reset style** restores the defaults. Removed series are listed at the bottom with **Restore**.

### Marks

Marks annotate the chart and are included in exported figures and sessions.

| Kind | Create with | Fields |
| --- | --- | --- |
| Region | **Region** (uses the selected range), or right-click → Add region from selection | Label, Start, End |
| Event | **Event** (at the selection start or the last cursor position), or right-click → Add event marker here | Label, X |
| Reference line | **Reference line** (at the mean of the first series), or right-click → Add reference line at this value | Label, Pane, Value |
| Point | Right-click a series → Add point annotation here | Label, Series, X, Y |

X values are typed in the axis' own format: clock time for timestamps (`12:35:23.7`), elapsed time for durations, plain numbers otherwise. Each mark has a colour, a zoom button and a delete button. Marks belong to the mode they were created in.

### Filters

Filters hide samples without changing the data. **Add filter**, then choose:

- **Keep inside** or **Exclude inside**.
- **Apply to**: *All plotted series* or one series.
- **X minimum**, **X maximum**, **Y minimum**, **Y maximum**: tick a bound to use it; it starts at the data's extent. Bounds are inclusive. Samples with missing values count as outside.

Hidden samples are left out of the chart and of the statistics. Each filter can be switched off with **Enabled**. Filters belong to the mode they were created in.

## New channels and formulas

**Data → New channel…** (Ctrl+N), the **{ }** button or right-click a channel → **New channel from this** opens the dialog.

- **Name** and **Unit**: the channel is called `Name [Unit]`.
- **Formula**: reference channels in braces, `{CPU Package Power} + {GPU Power}`. After `{`, matching channel names are suggested; Tab completes the first suggestion, or click one.
- **Templates** fill in a formula from the selected channels: *Sum*, *A − B*, *A ÷ B*, *Efficiency* (100 × first ÷ second, %), *Rate of change* (`gradient`), *Energy ∫ dt* (`cumtrapz`; W over seconds gives J), *Moving average* (`movmean`, 10 samples), *× 1000* and *÷ 1000*. Templates also suggest a name and unit.
- **Preview**: a sparkline and min/mean/max (with the unit) for the first checked source, or the error.
- **Add to every source that has these channels** (shown with several sources): add it everywhere it can be computed.

The new channel is plotted immediately. Formulas are recomputed when files are reloaded or followed, and are saved in sessions. Right-click a formula channel → **Edit formula…** to change or delete it.

### Formula syntax

| Element | Syntax |
| --- | --- |
| Channel | `{Name}` (without the unit) or `{Name [unit]}` |
| Numbers, text | `1.5`, `1e-3`, `'text'` or `"text"` |
| Constants | `pi`, `e`, `nan`, `inf`, `True`, `False` |
| Arithmetic | `+ - * /`, `//` (floor division), `%` (modulo), `**` (power). `^` is not an operator |
| Comparison | `< <= > >= == !=` (1 for true, 0 for false) |
| Logic | `and` / `&` / `&&`, `or` / `\|` / `\|\|`, `not` / `~`. These bind more loosely than comparisons, so `{A} > 5 & {B} < 3` works without brackets |

Precedence from loosest to tightest: `or`, `and`, `not`, comparisons, `+ -`, `* / // %`, unary minus, `**`.

Functions (the prefixes `np.`, `numpy.` and `math.` are accepted and ignored):

| Kind | Functions |
| --- | --- |
| Element-wise | `sin cos tan arcsin arccos arctan sinh cosh tanh exp log log10 log2 sqrt abs floor ceil round sign deg2rad rad2deg isnan`; also `asin acos atan radians degrees fabs absolute rint around` |
| Two arguments | `arctan2(y, x)`, `hypot(a, b)`, `power(a, b)`, `mod(a, b)`, `minimum(a, b)`, `maximum(a, b)` |
| Whole channel → one value | `mean std median sum min max` and the NaN-ignoring `nanmean nanstd nanmedian nansum nanmin nanmax` |
| Series | `cumsum(y)`, `gradient(y, x)`, `cumtrapz(y, x)` (running integral from 0), `movmean(y, n)` (centred moving mean over n samples), `where(condition, a, b)`, `clip(x, lo, hi)` |

Timestamps can be compared with dates written as text: `{Time} > '2023-03-18 12:36:00'`.

## Processing pipeline

**Data → Processing pipeline…** opens the list of processing steps. Steps run in order on every source. Filter, resample, bin and outlier steps change only what is plotted; mask, units and table steps change the tables (reloading restores the originals). When steps are active, the sidebar shows them as chips under **Processing**.

**+ Add step** adds:

| Step | What it does | Parameters |
| --- | --- | --- |
| Mask rows | Keep rows matching an expression | A [formula](#formula-syntax) condition, e.g. `{Time} > 5 and {CPU Package Power} < 150` |
| Filter | Smooth or frequency-filter signals | Moving average (window, default 100 samples), Low pass 1st order or High pass 1st order (cutoff frequency in cycles per X unit) |
| Resample | Change the sample coordinates | Every n (default 2), Linspace (start, end, number of points), Time-based (sample interval), Delta x (step, optional range), Replace, Insert or Remove (comma-separated X values) |
| Bin values | Average samples in equally spaced X bins | Bins (default 50), from, to |
| Remove outliers | Replace values far from the median with gaps | Median absolute deviations from the median (default 5) |
| Standardize units | Convert recognised units | SI, or Wind energy (kN, kW, rpm, deg) |
| Rename channels | Convert OpenFAST channel names | OpenFAST 2 → 3, OpenFAST 3 → 2, Fld → Aero, Aero → Fld |
| Nodal average | Average OpenFAST spanwise channels | Last part of the signal (fraction), Last N rotor revolutions, or Last N revolutions from rotor speed |
| Nodal concatenation | Combine OpenFAST spanwise channels into a new table | |
| Concatenate tables | Combine rows from the checked tables | Only channels present in every table |

Each step can be switched off, reordered or removed. **Save…** writes the steps as JSON; **Load…** replaces the steps with a saved file. **Apply** runs them. Saved pipelines also work with `--pipeline`, and the steps are saved in sessions.

## Analyze tools

The **Analyze** menu opens a floating window with four tools. It stays open while you work with the chart; close it with its × button. Each tool works on one **Series** and on **Samples** in the **Selection**, the **View** or **All**. Without a selection, View is used.

### Curve fit

Choose a **Model**: Polynomial (full) with an **Order** (0–12), Polynomial (partial) with **Exponents**, Exponential, Logarithmic, Gaussian, Gaussian with y-offset, Sinusoid, Power law, Weibull PDF, 2nd order impulse/decay and step responses (automatic or manual), Generator Torque, or **Custom formula** such as `{a} * exp(-x / {tau}) + {c}` (coefficients in braces, `x` is the X value). **Initial guesses** can be edited.

**Fit** shows R², RMSE, every coefficient and the iteration count. With a timestamp X, x is measured from the first fitted sample. **Show on chart** overlays the fit as a dashed line. **Copy results** copies them as text. **Add fit as table** adds the fitted curve as a new source (`Fit · <series>`). Fit results are not saved in sessions.

### Damping

For a decaying oscillation, **Estimate** finds the peaks and reports natural frequency, damped frequency and period, damping ratio ζ (with its range), logarithmic decrement and the number of peaks used. **Show envelope** draws the fitted envelopes. A warning appears when the peaks are not decaying.

### Fatigue

Rainflow counting and the damage-equivalent load. Set the **Wöhler exponent m** (default 3), **Equivalent period** (default 1 s), **Range bins** (default 100) and **Rainflow method** (WindAP, ASTM E1049 or fatpack), then **Compute**. Results are the damage-equivalent load, the number of counted cycles and the largest range. Ranges are peak-to-peak.

### Statistics report

Every statistic for one series: samples, minimum, maximum, mean, median, standard deviation, variance, RMS, P1, P5, P95, P99, peak-to-peak, absolute maximum, start and end values, change, least-squares slope, integral and integral mean, X moments (numeric X only), X at minimum and maximum, X span, mean sample spacing and turbulence intensity. **Copy report** copies it as tab-separated text.

## Table, Info and Fields views

Switch views with the top-bar buttons or Ctrl+1 to Ctrl+4. The statistics dock and inspector are shown only in Chart view.

### Table view

The raw rows of one source (choose it in the box when several are open). **Find a column…** filters the columns, not the rows. Missing values show as `—`. In Signals mode the table follows the chart cursor, and clicking a row moves the chart cursor to it.

### Info view

For each source: path, reader (format), table name, rows (and rows after a mask), columns, file size, modification time, sample rate, duration, whether it is in the chart, alignment, offset and mask; the reader notes; formula channels with their formulas; and processing steps.

### Fields view

Shown when a file contains 2D fields (VTK, TurbSim and other field formats). Files that contain only fields open there directly.

- **Field**: which field to show.
- **Color map**: viridis (default), plasma, inferno, magma, cividis, coolwarm, RdBu, jet, gray.
- **Range**: *Automatic*, *Common to all fields* or *Manual* (Min, Max).
- **Polar**: X is the angle (in **Degrees** or **Radians**) and Y the radius.
- **Export**: **Matrix as CSV…** or **Image as PNG…** (one pixel per cell, in the current colour map and range).

Hover the field to read the coordinates and value.

## Exporting

### Figures

**File → Export figure…** (Ctrl+E) opens a dialog with a live preview of exactly what will be saved.

| Option | Choices |
| --- | --- |
| Format | PNG (default), SVG (text converted to outlines), PDF (embedded fonts), EPS (embedded raster) |
| Size | Slide · 1920 × 1080, Document · 1600 × 1000 (default), Square · 1200 × 1200, Wide · 1200 × 600, or custom width and height |
| Resolution | 1×, 2×, 3× |
| Appearance | Light (default), Dark, Match app; Transparent background (PNG and SVG) |
| Include | Title, Legend, Annotations, Selected range |
| Labels | Title, Subtitle, X axis and Y axis text. Empty fields use the chart's labels |

**Save…** writes the file; **Copy to clipboard** copies the image. The figure shows the current view, panes, styles and marks. A side may not exceed 32,768 pixels and the image 100 million pixels.

**Copy figure** (Ctrl+Shift+C) copies the chart as an image without opening the dialog, using the dialog's last settings.

### Data

**File → Export data…** (Ctrl+Shift+E), or right-click a source → **Export data…**.

- **Rows**: **All rows**, **Visible range** or **Selection** (selection is preselected when a range is selected). Offsets and alignment are taken into account.
- **Columns**: **Plotted channels** (X plus every plotted channel) or **All columns**.
- **Format**: CSV (comma-separated), Parquet, OpenFAST binary (.outb) or Wind file (.wnd).

Rows removed by a **Mask rows** processing step are left out, and other table steps (such as unit standardisation) are included. Plot-only steps such as filters and resampling are not applied to exported data.

### Statistics

See [Copying](#copying) under Statistics.

### Python script

**File → Export Python script…** writes a script that reproduces the chart from the source files, including processing steps and formulas. Choose the **Library** (pyDatView, welib or OpenFAST toolbox), how **Data frames** are held (Dictionary, List or Variables), **One table per file**, and the amount of **Comments**. **Save…** or **Copy**. Running it needs Python and the chosen library.

### Without a window

`--export PATH` renders a figure (`.png`, `.svg`, `.pdf`, `.eps`) or a Python script (`.py`) and exits. Files are arranged as they are in the window: interleaved devices are split and separate recordings are aligned at their start. See [Command line](#command-line).

## Sessions

**File → Save session** (Ctrl+S) saves a `.dvsession` file with the open files and their formats, generated sources and how they were made, source names, offsets and alignment, formula channels, the X axis and plotted channels, the mode and its options, panes, styles, marks, filters, processing steps, the zoomed view, chart and figure options, statistics columns and Follow. Data is not stored; it is read from the files again. **Save session as…** (Ctrl+Shift+S) saves to a new file; **File → Open session…** opens one. Recent sessions appear on the welcome screen.

If a file cannot be found when a session opens, the problems button in the status bar lists it with **Locate…**: pick the file's new location and the session continues. Files that cannot be found stay in the session when it is saved again.

**Restore the last session** in Settings reopens the most recent session at startup, unless files are passed on the command line.

## Following live files

Turn on **Follow** in the status bar (or **View → Follow file changes**) to watch the open files. When a file changes, it is reloaded after it has been stable for half a second, formulas and processing are applied again, and if the view was showing the end of the data, it scrolls to keep the newest samples in view. **Follow files** in Settings turns it on at startup; `--follow` does so for one launch.

## Command palette

Ctrl+K (or Ctrl+Shift+P, or the button at the top right) opens the command palette. Type to search every enabled command, every channel (`Plot …` / `Hide …`) and every source (`Show source …` / `Hide source …`). Words match in order with letters skipped; `|` separates alternatives. ↑/↓ choose, Enter runs, Escape closes.

## Settings

**Edit → Settings…** (Ctrl+,). Settings are saved in `%APPDATA%\rustDatView\data\app.ron`.

| Setting | Default | Notes |
| --- | --- | --- |
| Theme | System | System follows the Windows app mode; or Dark, Light |
| Interface scale | 100 % | 80–200 % |
| Font | Inter (bundled) | Or any installed font |
| Series colors | Standard | Standard, Vivid, Color-vision friendly |
| Line width | 1.6 px | 0.5–6 px |
| Significant digits | 4 | Digits in readouts, legends and statistics (1–12) |
| Gaps: Break lines after | On, 10× | Multiple of the median sample interval |
| Show: Channel sparklines | On | |
| Show: Overview strip | On | |
| Dates: Day first when ambiguous | Off | Applies to files opened or reloaded afterwards |
| Startup: Follow files | Off | |
| Startup: Restore the last session | Off | |
| Recent items | | **Clear** forgets recent files and sessions |

**Reset to defaults** resets everything except recent items.

## Command line

```text
rustDatView.exe [OPTIONS] [FILE ...]

  -f, --format NAME     Reader format (default: auto)
  --dayfirst            Interpret ambiguous dates as day/month/year
  --mode MODE           Signals/Regular, Distribution/PDF, Spectrum/FFT,
                        Normalized/MinMax, Compare, Polar
  --x INDEX             Zero-based X column (including Index)
  --y INDEX[,INDEX]     Y column indices
  --pipeline PATH       Load a processing pipeline JSON
  --plot-options JSON   Serialized dv-plot PlotType options
  --session PATH        Open a .dvsession workspace
  --follow              Watch input files for changes
  --export PATH         Export SVG, PNG, PDF, EPS or Python without a window
  -h, --help            Display help
  -V, --version         Display version
```

- `--mode` is not case-sensitive.
- `--x` and `--y` count columns from zero, including an `Index` column when the reader adds one.
- `--plot-options` sets a mode together with its options, for example `{"Pdf":{"bins":50,"smooth":true}}`. The other forms are `"Regular"`, `{"Fft":{…}}`, `{"MinMax":{…}}` and `{"Polar":{…}}`, with every option field given.
- `--session` can be combined with files; the files are added after the session has loaded. `--mode`, `--plot-options`, `--pipeline` and `--follow` override the session.
- Use `--` before file names that begin with a dash.

Exit codes: 0 on success, 2 for invalid arguments, 1 when `--export` fails (`Export failed: …` on standard error). Shells and scripts wait for the command to finish, so the exit code and output can be checked. Started from a terminal without `--export`, the terminal waits until the window is closed.

Examples:

```text
rustDatView.exe examples\signals.csv --mode Spectrum --export spectrum.png
rustDatView.exe examples\cpu-gaming-session.csv --y 2,3 --export power-and-temperature.pdf
rustDatView.exe --session tuning.dvsession --export tuning.svg
```

## Keyboard shortcuts

F1 or ? shows this list in the app.

| Shortcut | Action |
| --- | --- |
| Ctrl+O | Open file… |
| Ctrl+Shift+O | Add files… |
| F5 | Reload |
| Ctrl+S | Save session |
| Ctrl+Shift+S | Save session as… |
| Ctrl+E | Export figure… |
| Ctrl+Shift+C | Copy figure (copies statistics when the statistics dock was clicked last) |
| Ctrl+Shift+E | Export data… |
| Ctrl+Q | Quit |
| Ctrl+F or / | Find channel |
| Ctrl+, | Settings… |
| Ctrl+1, Ctrl+2, Ctrl+3, Ctrl+4 | Chart, Table, Info, Fields |
| Alt+1 … Alt+6 | Signals, Distribution, Spectrum, Normalized, Compare, Polar |
| Ctrl+B | Show or hide sidebar |
| Ctrl+I | Show or hide inspector |
| Ctrl+J | Show or hide statistics |
| Home or 0 | Fit all (chart) |
| Ctrl+K or Ctrl+Shift+P | Command palette… |
| Ctrl+N | New channel… |
| F1 or ? | Keyboard shortcuts |

Chart keys (pointer over the chart): ←/→ pan, +/− zoom, Alt+←/Alt+→ previous/next view, Escape clears the selection. Escape also closes dialogs and the command palette.

Single-key shortcuts (/, ?, 0, F-keys) and Alt shortcuts are ignored while typing in a text field.

Open with format…, Open session…, Export Python script…, Follow file changes, Processing pipeline…, the Analyze tools and About have no shortcut; use the menus or the command palette.

## Limitations

- `--export` uses default preferences (colours, line width, digits) and default figure options (PNG size 1600 × 1000, light). To export an arranged view, save a session and use `--session`.
- `--export` fails if loading reports any problem, including warnings that the window would only show as a notification.
- `--dayfirst` is stored in Settings; turn **Day first when ambiguous** off again to undo it.
- Energy is only computed in Signals mode with a time X axis and a power unit.
- Split by column supports at most 64 distinct values.
- Sources can only be reordered with **Move up** and **Move down**.
- Ctrl+click in the channel list acts like a plain click.
- The status bar summarises only the first source.
- Fit results are not saved in sessions; re-run the fit after opening one.
