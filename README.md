# Mini Imp

Native Windows app for N-up imposition. Add PDF pages and images, arrange them in a grid with margins, gutters, and crop marks, then export a vector PDF or print it.

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

Image previews and PDF export work without it. Export keeps vectors; the print path rasterizes that same PDF at up to 300 DPI and fits it to the printer's printable area.
