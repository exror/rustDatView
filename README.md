# rustDatView

Rust-native desktop implementation of [pyDatView](https://github.com/ebranlard/pyDatView): open tabulated scientific data, select channels, plot and compare signals, transform tables, and export results. The desktop application uses egui rather than wxPython; Python is not required to run it.

## Build and run

Install stable Rust with [rustup](https://rustup.rs/), then:

```sh
cargo build --release -p datview --locked
cargo run --release -p datview -- examples/signals.csv
cargo run --release -p datview -- examples/field_lzma.vti
```

On Windows, the standard MSVC toolchain requires Visual Studio C++ Build Tools and the Windows SDK. A GNU toolchain with a complete MinGW installation is also supported. Linux desktop builds need development packages for the display backend (for Debian/Ubuntu: `libxkbcommon-dev`, `libwayland-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`). macOS builds use the normal Rust and Xcode Command Line Tools setup.

The executable is `target/release/datview.exe` on Windows (`target/release/datview` elsewhere). Double-click it, drag files onto its window, or run it with filenames:

```powershell
.\target\release\datview.exe .\examples\motherboard-power.csv
```

The repository does not bundle compilers, private build configuration, input files from a user's machine, or credentials.

## Workflow

- Open files using **File → Open**, the command line, or drag-and-drop. Ctrl-drop appends instead of replacing. Choose **Auto format** or a particular reader.
- Select tables and X/Y channels. Numeric/datetime Time, Elapsed Time and Timestamp channels are preferred for X, with Index as the fallback. **Auto**, **Common columns**, and **Independent tables** support matching channels across reordered tables.
- Select **Regular**, **PDF**, **FFT**, **MinMax**, **Compare**, or **Polar**. Configure transform options, logarithmic axes, lines/points/steps, subplots, synchronized X axes, limits and styling.
- Table operations include rename, reload, sort, reorder, transpose, merge on X with interpolation, vertical concatenation and CSV/Parquet/OpenFAST binary export.
- Formula columns use names without units in braces: `{Signal} * 2`, `sin({Time} * 2 * pi)`. Masks support numeric/date/string comparisons and `&`, `&&`, `|`, `||`, `and`, `or`, `not`. Examples: `({Time} > 1) & ({Time} < 8)`, `{Name} == 'rotor'`, `['rotor' in str(x) for x in {Name}]`. Row comprehensions accept the bound variable, literals, comparisons, string membership, `str()` and boolean operators; they do not execute arbitrary Python.
- The pipeline provides masking, filtering, resampling, binning, outlier rejection, unit conversion, OpenFAST channel renaming, nodal averaging and nodal concatenation. Actions can be removed and reapplied on reload; configurations can be saved as JSON.
- Statistics include original/transformed signal summaries and integrals. Two-click measurements expose interval statistics. Tools provide model fitting, logarithmic-decrement damping estimates and rainflow equivalent loads.
- **2D fields** exposes available scalar fields, coordinate axes, colormaps, automatic/common/manual color ranges, polar projection and CSV matrix export.
- Export figures as SVG, PNG, PDF or EPS. Export Python plotting scripts with transform/pipeline configuration. Generated scripts require Python and the selected scientific library flavor; exporting a script does not execute it.
- Preferences and pipeline options persist through egui's application storage. Loaded filenames and table data are not persisted.

## Interactive telemetry figures

`examples/motherboard-power.csv` is a **synthetic** motherboard trace: shared CPU core/GFX, SOC and memory power, clock and temperature, with a workload interval and one deliberately injected power spike.

1. Open your CSV or another supported tabular log. Check the X dropdown: a recognized time column is selected even when it is not the first column. Choose **Index** explicitly for sample-number plots. Units remain those in your file; milliseconds are not silently converted to seconds.
2. Use the series search to find channels. Matching is case-insensitive and fuzzy: `cpupwr` matches separated letters; `soc power` requires both tokens; `core | gfx | memory` finds alternatives. Hidden selections stay selected. **Select matches** adds channels, **Deselect matches** removes only matches, and **Clear all** resets the Y selection. **Power rails**, **Clocks** and **Temperatures** provide starting queries.
3. Open **Plot studio → Annotate**. Enter a label, choose its highlight color, then select **Point** and click near a sample, or select **Region** and click its start and end. Point markers snap to visible samples. Edit labels, colors and region bounds in the inspector. **Esc** cancels a pending annotation. Pan/zoom and double-click-to-fit remain available.
4. Under **Filter**, target all selected series or one particular series. Supply any combination of X/Y minimum and maximum, then **Keep inside** or **Exclude inside**. Boundaries are inclusive; multiple enabled rules are combined. Exclusions leave gaps rather than drawing misleading connecting lines. For example, keep core/GFX power below 150 W without changing SOC or memory. Rules can be disabled or removed.
5. Under **Share**, set title, axis labels, resolution, grid, legend and subplots. **Fonts & appearance** offers searchable installed font families, live interface preview, light/dark themes, and separate interface/export font sizes. Fonts are loaded from your system, not bundled into this repository. Restart after installing a new font.
6. **Save annotated image** produces a clean figure without UI controls. **Copy image to clipboard** pastes the same figure into chat or an image editor. PNG is convenient for messages; SVG/PDF retain vector geometry and resolve font glyphs so recipients need not install your chosen font. EPS embeds a raster image to preserve typography and highlights.

Display filters are non-destructive and use the current plot mode's X/Y values before logarithmic/flipped display axes. Source tables, statistics and analysis tools are unchanged; use a pipeline mask when the filtered values should also affect analysis. Annotations and display rules are scoped to the plot mode and **session-only**: export your figure before closing or replacing the loaded files. Python script export reproduces the analysis pipeline, not these manual presentation edits. Appearance and figure settings persist.


## Command line

```sh
# Multiple files
cargo run -p datview -- first.csv second.outb

# Export without opening a desktop window
cargo run -p datview -- examples/signals.csv --mode FFT --export spectrum.png
cargo run -p datview -- examples/signals.csv --mode PDF --export probability.svg
cargo run -p datview -- examples/signals.csv --y 2,3 --mode Compare --export comparison.pdf
cargo run -p datview -- examples/signals.csv --export reproduce.py

# Full reader list and options
cargo run -p datview -- --help
```

X/Y indices are zero-based and include the inserted `Index` column. `--pipeline PATH` loads pipeline JSON; `--plot-options JSON` accepts the serialized `dv-plot::PlotType` configuration. Use `--` before filenames beginning with a dash.

## File formats

The registry includes CSV/delimited text, Excel, Parquet, NetCDF, MATLAB, safe tabular pickle decoding, TDMS, VTK, OpenFAST input/output/linearization/summary/wind files, TurbSim, ROSCO, HAWC2, HAWCStab2, FLEX, Bladed, BModes, GNUplot, Tecplot, CACTUS, Mann boxes and RAAW MAT files. Individual readers return their table names, units and available scalar fields rather than coercing everything into one matrix. Explicit format errors are surfaced in the application.

### Compatibility boundaries

This is a Rust application, not a Python interpreter. Arbitrary Python objects inside pickle files and dynamically loaded user Python plugins are not executed. Supported pickle tables are decoded by an inert parser. Python/matplotlib demo functions and standalone wind-farm simulation-generation helpers outside the desktop workflow are not included. External AMRWind conversion helpers require a reader absent from the referenced upstream checkout.

The core table representation supports numeric, text and datetime columns; it does not represent arbitrary Arrow nested objects or pandas extension objects. Linearization matrix row labels are available through the file object's `matrix_row_labels` API instead of a pandas index. Subformat restrictions are reported as errors, never substituted with fake data.

NetCDF supports classic/CDF-2 and NetCDF-4 root variables up to two dimensions, with Gregorian CF time decoding; CDF-5 and non-Gregorian calendars are rejected. MATLAB supports level-5 and v7.3 numeric/logical/text/cell/structure values; sparse matrices, object classes and recursive references are rejected. TDMS supports contiguous/interleaved numeric channels, strings and timestamps in both byte orders; DAQmx scaler streams and extended/complex samples are rejected. VTK XML enables zlib, LZ4 and LZMA compression.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `dv-core` | Ordered columns, tables, table operations, formulas, file-object contract |
| `dv-tools` | Spectral/signal analysis, statistics, fitting, damping, fatigue, palettes |
| `dv-io` | File registry, readers/writers, OpenFAST nodal/field postprocessing |
| `dv-plot` | Plot transforms, comparisons, plugin actions, pipelines, script export |
| `datview` | Desktop UI, selection, figure rendering and command-line export |

```sh
cargo test --workspace --locked
cargo build -p datview --locked
```

CI is configured to build and test the workspace on Linux, Windows and macOS. Local smoke checks exercise the executable and independently generated scientific file formats; unit tests alone do not establish desktop rendering correctness. The checked-in MATLAB7.3 fixture is synthetic HDF5 data created with h5py: numeric samples, UTF-16 text and cell/structure object references behind a 512-byte MATLAB userblock.

## License and provenance

MIT; upstream copyright is retained in `LICENSE`. pyDatView reference revision: `07187cbaf8c76090f2b2eb89278bdde99ac4ef4d`. Matplotlib/ColorBrewer palette notices and complete terms are included in `THIRD_PARTY_NOTICES.md` and `licenses/`.
