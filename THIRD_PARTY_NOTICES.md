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

Rust dependencies remain under their respective package licenses; their sources are fetched through Cargo and are not vendored in this repository.
