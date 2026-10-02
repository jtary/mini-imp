# Mini Imp

<p align="center">
  <img src="misc_assets/imp.png" alt="Mini Imp, a red imp in a green apron with a pencil behind one ear" width="280">
</p>

Native Windows app for N-up imposition. Add PDF pages and images, arrange them on a sheet, then export a vector PDF or print it.

The window follows the Windows light or dark theme. Sources sit on the left, the sheet preview in the center, and sheet settings on the right.

## Sources

Add files from the command bar, or drop them on the sources list. Each PDF page becomes its own source. Images can be PNG, JPEG, TIFF, BMP, WebP, or GIF.

For each source you can:

- Repeat it with an instance count
- Reorder or remove it
- Turn it clockwise in 90° steps; the fit, preview, and export all follow that turn
- Set a target trim size, which stays as entered and has to fit in the cell

## Sheet

Sheet sizes are Letter, Legal, Tabloid, A4, A3, SRA3, or a custom size, in portrait or landscape. Measurements can be millimetres or inches.

The grid sets columns and rows, and each cell can contain or fill the artwork. Signature order is available for folded sections. Crop marks, margins, gutters, and bleed are all adjustable. The preview warns when bleed or crop marks would be clipped, and when a target size does not fit the cell.

## Build

```
cargo run
```

Release builds hide the console window:

```
cargo build --release
```

The executable is `target\release\mini-imp.exe`.

## Pdfium

PDF page previews and printing need a 64-bit `pdfium.dll` next to the executable. Download the Windows x64 build from [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases) and copy `pdfium.dll` beside `mini-imp.exe` (or into the working directory while using `cargo run`).

Image previews and PDF export work without it. Export keeps vectors. Printing rasterizes that same PDF at up to 300 DPI and fits it to the printer’s printable area.
