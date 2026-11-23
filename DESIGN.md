# PDF Wizard — interface redesign

## Brief

**Subject:** a desktop PDF page arranger (Rust + egui). People open scans, reports, and image
files, reorder pages, rotate/crop them, and export a clean PDF or booklet.

**Audience:** office and print-shop users, teachers, researchers, designers. They know PDFs, not
necessarily this app.

**Job:** make the first five minutes obvious, keep repeated arranging fast, and make exports safe
and predictable.

**Constraints:** native egui app, no web stack, must keep keyboard shortcuts, touch targets,
undo history, background loading/export, and the existing PDF engine. The redesign is a UI and
interaction change, not a rewrite of PDF handling.

**Assumptions:** English interface; desktop-first with touch support; no account, cloud, or
onboarding flow. Current sample documents are user-supplied.

## Concept

**Quiet bindery.** A page arranger should feel like a well-lit workbench for paper: warm stock,
precise rules, a deep verdigris accent, folio numbers, and nothing shouting. The interface
borrows the rules of print production, not the visual language of a dashboard.

- **Idea:** arranging pages is binding a document, so the app should feel like a calm bindery
  bench rather than a file manager.
- **Anchor:** the subject itself is sheets of paper that get ordered, turned, and bound. The
  concept could not be moved unchanged to a chat app or a music player.
- **Emotion:** the first three seconds should feel quiet, competent, and unhurried.
- **Motion verb:** settle like paper. Cards lift a hair and settle; the selection tray rises
  and rests; nothing bounces.
- **Screenshot frame:** the empty workspace, with the stacked-sheet illustration, serif
  headline, and the dotted bench surface.
- **Signature moment:** the selection tray: a floating, paper-weight strip that rises from the
  bottom edge when pages are selected, carrying every action for the selection in one reach.

### Concepts considered

| Concept | One sentence | Signature | Fit / originality / build / portability |
|---|---|---|---|
| Quiet bindery | A print workbench with warm paper, folio marks, and a verdigris ink accent. | A selection tray that rises like a paper slip. | 5 / 4 / 5 / 2 |
| Flight deck | A dark, instrument-panel workspace driven by a command palette. | Command palette with live page thumbnails. | 3 / 4 / 3 / 4 |
| Blueprint atelier | Drafting-paper grid and measure marks around the page grid. | Drag guides that measure the gap in millimetres. | 3 / 2 / 4 / 3 |

Chosen: **Quiet bindery.** It is the only one that comes directly from the object being edited
and stays quiet enough for repeated office work. Flight deck is efficient but cold; blueprint is
charming but its measurement metaphor is decoration for most users.

## Art direction

### Palette (4–6 named values by role, plus semantics)

Light — **Paper**

| Role | Hex | Use |
|---|---|---|
| Ground | `#F1EDE4` | window, canvas, dotted bench |
| Surface | `#FFFCF6` | cards, header, sidebar document block |
| Ink | `#1E1C18` | primary text |
| Muted | `#6F685C` | secondary text, metadata |
| Line | `#DCD4C5` | hairlines, borders |
| Accent | `#1F5C4A` | primary actions, selection, crop guides |
| Accent soft | `#E2EDE7` | selected card tint, quiet chips |
| Brass | `#9A7438` | logo fold, rare ornament |

Dark — **Night bindery**

| Role | Hex | Use |
|---|---|---|
| Ground | `#141512` | window and canvas |
| Surface | `#201F1B` | cards, header, sidebar blocks |
| Ink | `#F3EEE3` | primary text |
| Muted | `#A49C8D` | secondary text |
| Line | `#35332C` | hairlines |
| Accent | `#7FB7A1` | actions and selection |
| Accent soft | `#22322B` | tinted selection |
| Danger | `#D98378` | destructive text |

No pure black or pure white; paper thumbnails stay `#FFFFFF` because a PDF page is white.

### Type

- **Instrument Sans** (OFL, static 400/500/600) — UI, labels, buttons, body.
- **Newsreader** (OFL, 14 optical size) — wordmark, empty-workspace headline, dialog titles.
- **Lucide** icon font (ISC) — a single consistent 1.5-stroke icon set, used as a font fallback so
  icons align with labels and inherit button states.
- **Hack** (bundled with egui) — folio numbers, zoom value, shortcut keys.

Scale: 10.5 label / 11.5 caption / 13 button / 14 body / 20 wordmark / 34–40 empty-state
display. Body line height ≈ 1.5. Display tracking −0.5 px; small uppercase labels tracking +0.6.
Body text is never below 11.5 px except keyboard chips and unit metadata.

### Grid and rhythm

- 8 px base spacing. Window: 58 header / 30 status, 272 sidebar, canvas gutters 24.
- Page grid: card width is the zoom value, default 172 px; gap 18 px; columns computed from the
  available width (1 minimum, usually 4 at the default window size).
- Sections in the sidebar are separated by 20 px and hairline rules; no boxed section inside a
  boxed section.
- Alignment rule: text aligns to a 20 px sidebar gutter; folio numbers and percentages align
  right or center on their own column. Display type is optically hung by 1 px, not boxed.
- Section rhythm varies: document card → quiet action stack → contextual section → export.
  The canvas is the loudest area by being emptiest.

### Shape and motion

- Shape: 2 px on paper, 8 px on controls, 10 px on cards, 14 px on modals and the tray.
- One easing family: ease-out / cubic. Durations: 120 ms hover/press, 180 ms selection tray,
  200 ms empty-state transition. No idle animation: this is a work tool, and a permanently
  moving canvas competes with the pages. Hovers, the tray, and the loading spinner carry life.
- Motion language: lift max 2 px; selection tint fades, border does not jump; tray translates
  12 px and fades. Never animate layout properties.

### Composition

Header
```
[☰] [mark] PDF Wizard   File  Edit  Pages  View  Help        [undo][redo] | [sun] [?]  [Save PDF]
```

Workspace
```
+----------------------+--------------------------------------------------------------+
| DOCUMENT             |  Pages (12)  · 3 selected            [−] 100% [+]           |
| report.pdf           |--------------------------------------------------------------|
| 12 pages · 2 files   |  dot-grid bench                                              |
|                      |                                                              |
| ADD PAGES            |   +---------+  +---------+  +---------+  +---------+        |
| [ folder  Add pages ]|   | [✓] #03 |  | [ ] #04 |  | [ ] #05 |  | [ ] #06 |        |
| [image Images][blank]|   |  paper  |  |  paper  |  |  paper  |  |  paper  |        |
|                      |   | caption |  | caption |  | caption |  | caption |        |
| SELECTION            |   +---------+  +---------+  +---------+  +---------+        |
| 3 pages selected     |                                                              |
| [rot L][rot R]       |                                                              |
| [crop ][duplicate]   |           [ 3 selected | ← → | ↺ ↻ | crop | ... ]           |
| [extract][delete]    |                                                              |
|                      |                                                              |
| EXPORT               |                                                              |
| [ save  Save PDF   ] |                                                              |
| [extract][split][book]                                                              |
+----------------------+--------------------------------------------------------------+
| Ready. Drag pages to reorder.                                 12 pages · 2 files     |
+-------------------------------------------------------------------------------------+
```

Empty workspace
```
+----------------------+--------------------------------------------------------------+
| sidebar (export off) |  Pages                                                        |
|                      |   +--------------------------+   +-----------------------+   |
|                      |   | Give every page          |   |   [stacked sheets]    |   |
|                      |   | its place.               |   |   [illustration]      |   |
|                      |   | Open PDFs and images...  |   |                       |   |
|                      |   | [ Choose files ] [Blank] |   |   A4 · 2 sources      |   |
|                      |   +--------------------------+   +-----------------------+   |
|                      |        or drop files anywhere                                 |
+----------------------+--------------------------------------------------------------+
```

Dialogs share one language: dimmed bench, 14 px surface, serif title, Instrument Sans body,
hairline separators, ghost Cancel + accent primary action.

### Signature and micro-interactions

| Role | Treatment |
|---|---|
| Primary CTA | Accent fill, subtle deepen on hover, no movement; arrow/save icon included. |
| Secondary button | Surface fill, line stroke; stroke darkens and fill warms on hover. |
| Ghost / icon button | Transparent at rest, accent-soft on hover, accent icon. |
| Page card | 2 px lift + soft shadow on hover; checkbox and accent ring on selection; grip deepens. |
| Reorder | 2 px accent insertion line with round caps; auto-scroll near the canvas edge. |
| Selection tray | Rises 12 px with a 180 ms ease-out; icon actions answer with a soft accent fill. |
| Inline links | Underline at rest; underline thickens on hover. |
| Destructive | Danger text on danger-soft hover; no filled red button except replace confirmation. |

## Interaction and information architecture

- The old hidden right “Tools” drawer is gone. A persistent, collapsible left bench holds the
  three jobs: add pages, act on the selection, export. It becomes contextual rather than a wall
  of buttons.
- A classic menu bar exposes the long tail (select odd/even, overlays, booklet, metadata) so the
  sidebar can stay short.
- “Add pages” is the primary file action. It appends to the current arrangement. “Open and
  replace” is explicit in File, and confirmed when pages exist.
- Selection: click selects; Shift-click selects a range; Ctrl/Cmd-click or the checkbox toggles.
  No separate select mode. Drag any card to reorder. No separate reorder mode.
- The selection tray duplicates the most-used selection actions near the pointer while the
  sidebar holds the full set, including crop and delete. Tooltips state disabled reasons.
- Export defaults to a useful filename derived from the source file, for example
  `report-arranged.pdf`.
- Keyboard map is unchanged where possible; add F1 shortcuts and Ctrl+B sidebar toggle. All
  shortcuts appear in Help → Keyboard shortcuts and in tooltips.
- The window title shows the document and page count; the status bar reports the latest action,
  with success and error tones.

## Accessibility and states

- Native focus rings on all custom cards and controls; Tab reaches cards and buttons; Space or
  Enter selects a focused card. Checkboxes expose their toggled state through the painted check
  and accessible label.
- Focus order: header actions → sidebar → canvas toolbar → page grid → tray. Skip link is not
  meaningful in a native desktop app; keyboard access starts at the first header control.
- Reduced motion: egui animation time is globally configurable; hover/selection still work
  without transitions. No content is hidden at rest or under any theme.
- Contrast: every text token is checked against its background at AA or better; tests compute
  WCAG ratios for both themes.
- Text over GL/images: not applicable. Page images always sit on a white paper rectangle, so
  captions never sit on PDF content.
- Busy, empty, loading, error, and no-preview states are designed, not left to defaults.
- Fixed UI never covers the last row: the grid reserves bottom space while the tray is open.

## Genericness test

A generic model would produce an Ant Design blue toolbar, a hidden settings drawer, icon-less
buttons, and a centered “drag files here” card. This redesign changes all four: a named warm
paper palette, a persistent bench, bespoke line icons, and a two-column empty state with a drawn
stack of sheets. The Deliberately avoided defaults: purple AI gradients, glassmorphism, cream +
terracotta editorial, acid green on black, three equal feature cards, decorative 01/02/03.
