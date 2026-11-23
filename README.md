# PDF Wizard

A pure-Rust, single-binary, cross-platform (Linux, macOS, Windows) PDF page arranger inspired by [PDF Arranger](https://github.com/pdfarranger/pdfarranger).

## Features

- Open/append multiple PDFs, import images (PNG/JPEG/WEBP) as pages, drag files onto the window
- Thumbnail grid with zoom, click / Ctrl+click / Shift+click selection, drag-and-drop reordering
- Rotate, duplicate, delete, insert blank page, crop, scale, overlay/underlay, copy/cut/paste
- Select all/none/invert/odd/even/same source; reverse order; swap odd/even pages
- **Extract selected pages** to one PDF or to separate files (`<name>_page_<n>.pdf`); split all pages into a folder
- Booklet generation (2-up saddle-stitch), metadata editing, undo/redo, light/dark theme
- Export edits PDF objects directly (via `lopdf`); pages are never rasterized on save

## Build

```bash
cargo build --release
# binary: target/release/pdf-wizard (pdf-wizard.exe on Windows)
```

Requires a recent stable Rust toolchain. The UI uses OpenGL (via `glow`), and the Linux build uses X11/Wayland at runtime (no GTK or other PDF libraries needed).

## Interface

- **Left bench** (collapsible with `Ctrl+B`): page counts, selection helpers, transform, page, and export actions. The classic menu bar holds less common actions.
- **Add pages** appends to the current document. **Open and replace** (`Ctrl+Shift+O`) confirms before clearing the bench, undo history, and copied pages.
- **Selection tray**: when pages are selected, a floating tray appears at the bottom of the canvas with move, rotate, crop, duplicate, export, and delete actions.
- Drag any page card to reorder; click, `Ctrl`-click, or `Shift`-click to select. `F1` lists keyboard shortcuts.

## Usage

```bash
pdf-wizard [file.pdf ...]
```

## Known limitations

- Password-protected PDFs are rejected with an error.
- Page annotations/links are not carried into exported files.
- Thumbnails use the pure-Rust `hayro` rasterizer; unusual PDFs may show "no preview" (export is unaffected).

