# Third-party notices

## pyDatView

Rust algorithms and file readers are derived from [pyDatView](https://github.com/ebranlard/pyDatView), reference revision `07187cbaf8c76090f2b2eb89278bdde99ac4ef4d`.

Copyright 2019 E. Branlard. MIT terms are retained in `LICENSE`.

## Matplotlib colormap data

`crates/dv-tools/src/colors.rs` contains palette data derived from Matplotlib 3.9.0 `_cm.py` and `_cm_listed.py`.

Copyright (c) 2012- Matplotlib Development Team; All Rights Reserved.

Changes: Python segmented/listed palettes were converted to precomputed Rust RGB lookup tables. No Matplotlib runtime is included. The Matplotlib license agreement is retained in `licenses/MATPLOTLIB.txt`.

The magma, inferno, plasma and viridis scientific palettes were released under CC0 by Nathaniel J. Smith and Stefan van der Walt; viridis also credits Eric Firing. Other palette data is not represented as CC0. See [BIDS colormap provenance](https://github.com/BIDS/colormap/blob/master/colormaps.py).

ColorBrewer palette data: Copyright (c) 2002 Cynthia Brewer, Mark Harrower, and The Pennsylvania State University. Apache License 2.0; notices and complete terms are retained in `licenses/COLORBREWER.txt` and `licenses/APACHE-2.0.txt`.

## Bundled fonts and icons

- Inter Regular and SemiBold, Copyright (c) 2016 The Inter Project Authors, are distributed under the SIL Open Font License 1.1; see `licenses/INTER-OFL.txt`. The bundled fonts are subsetted and their tabular figures are enabled.
- JetBrains Mono Regular, Copyright 2020 The JetBrains Mono Project Authors, is distributed under the SIL Open Font License 1.1; see `licenses/JETBRAINS-MONO-OFL.txt`. The bundled font is subsetted.
- Phosphor icons, Copyright (c) 2020–2021 Phosphor Icons, are distributed under the MIT License; see `licenses/PHOSPHOR-MIT.txt`. The regular icon font is bundled.
- Phosphor icon codepoints are taken from egui-phosphor 0.14.0, Copyright (c) 2023 Romet Tagobert, under the MIT License; see `licenses/EGUI-PHOSPHOR-MIT.txt`.

Rust dependencies remain under their respective package licenses; their sources are fetched through Cargo and are not vendored in this repository.
