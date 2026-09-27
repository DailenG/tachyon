# Tachyon editor tokens and component styling — proposal

**Status: proposal only.** No editor code or runtime assets change in this PR. Tachyon remains a single-pane GPUI block-swap Markdown editor. Values below are compiled-in solid colours except the existing alpha selection and find highlights. No font file, runtime SVG parser, animation, or new screen is proposed.

## Token values

| Role | Light | Dark | Application |
| --- | --- | --- | --- |
| `surface.canvas` | `#FAFBFE` | `#0E1623` | Editor background and content column |
| `surface.raised` | `#F1F5F9` | `#192434` | Raw editing card, find bar, picker, prompt |
| `surface.code` | `#E7EDF5` | `#131E2E` | Inline and fenced code |
| `surface.scrim` | `#172335` | `#080C14` | Opaque keyboard-prompt backing only; prompt text lives on raised |
| `text.primary` | `#172335` | `#EBF2FA` | Body text |
| `text.muted` | `#42566C` | `#ABBDD0` | Secondary text and metadata |
| `text.link` | `#1356A4` | `#8BC3FF` | Link text; same solid blue as accent.primary |
| `text.on_accent` | `#FFFFFF` | `#0E1623` | Text only over accent.primary solid fill |
| `border.subtle` | `#B5C2D0` | `#40526A` | Rules, tables, quote bar |
| `border.control` | `#66798F` | `#6F849B` | Input and checkbox boundaries |
| `border.focus` | `#1356A4` | `#8BC3FF` | Visible 1 px focus boundary |
| `accent.primary` | `#1356A4` | `#8BC3FF` | Links and selected-row edge; single solid blue per theme |
| `editing.selection` | `#1356A42B` | `#8BC3FF33` | Existing alpha highlight; base blue matches accent.primary |
| `editing.find_match` | `#E8AB4138` | `#BE844730` | Existing alpha highlight; 1 px underline distinguishes matches |
| `editing.find_current` | `#E8AB4154` | `#BE84474D` | Existing alpha highlight; 1 px outline distinguishes current |
| `editing.caret` | `#172335` | `#EBF2FA` | Steady 1 px caret |
| `syntax.keyword` | `#8B2A54` | `#F398C4` | Code tokens |
| `syntax.string` | `#286430` | `#A9D28B` | Code tokens |
| `syntax.comment` | `#45596E` | `#ABBDD0` | Code tokens |
| `syntax.number` | `#87400F` | `#E4B17D` | Code tokens |
| `syntax.function` | `#235892` | `#8BC3FF` | Code tokens |
| `syntax.type` | `#6A3B8C` | `#D3AAFF` | Code tokens |
| `syntax.math` | `#693C91` | `#C6A2ED` | Math text: restrained violet |

Semantic mapping to today’s `Theme` fields: `background` → canvas, `raw_background` → raised, `code_background` → code, `foreground` → primary, `muted` → muted, `accent` → accent.primary/link, `quote_bar` and `rule` → subtle, `selection`/`find_match`/`find_current`/`cursor` → the editing roles, `syntax[0..5]` and `math` → corresponding syntax roles. New control/focus/on-accent/scrim roles are proposed tokens, not code changes. Reuse system fonts and existing sizing unless separately measured.

## Contrast measurements

WCAG 2.x contrast ratios computed from relative sRGB luminance, rounded to 0.01. Columns are **canvas / raised / code**. Every text and syntax token below is checked on all three possible surfaces, even where the current component uses fewer. Minimums: **4.5:1 text**, **3:1 non-text boundaries and caret**. The slight rounding in this table does not change pass/fail.

### Light

| Text token | Hex | Canvas | Raised | Code |
| --- | --- | ---: | ---: | ---: |
| `primary` | `#172335` | 15.28:1 | 14.43:1 | 13.42:1 |
| `muted` | `#42566C` | 7.30:1 | 6.90:1 | 6.42:1 |
| `link` | `#1356A4` | 7.01:1 | 6.62:1 | 6.16:1 |
| `keyword` | `#8B2A54` | 7.94:1 | 7.50:1 | 6.97:1 |
| `string` | `#286430` | 6.87:1 | 6.49:1 | 6.03:1 |
| `comment` | `#45596E` | 6.98:1 | 6.59:1 | 6.13:1 |
| `number` | `#87400F` | 7.33:1 | 6.92:1 | 6.44:1 |
| `function` | `#235892` | 7.04:1 | 6.65:1 | 6.19:1 |
| `type` | `#6A3B8C` | 7.75:1 | 7.32:1 | 6.81:1 |
| `math` | `#693C91` | 7.64:1 | 7.22:1 | 6.71:1 |

| Non-text token | Hex | Canvas | Raised | Code |
| --- | --- | ---: | ---: | ---: |
| `border_control` | `#66798F` | 4.32:1 | 4.08:1 | 3.80:1 |
| `border_focus` | `#1356A4` | 7.01:1 | 6.62:1 | 6.16:1 |
| `caret` | `#172335` | 15.28:1 | 14.43:1 | 13.42:1 |

`text.on_accent` appears **only on** the solid `accent.primary` fill: `#FFFFFF` / `#1356A4` = **7.25:1**. It is not used on canvas, raised, or code; for those surfaces use primary/muted/link above. The accent itself is 7.01:1 on canvas, 6.62:1 on raised, and 6.16:1 on code.

### Dark

| Text token | Hex | Canvas | Raised | Code |
| --- | --- | ---: | ---: | ---: |
| `primary` | `#EBF2FA` | 16.08:1 | 13.86:1 | 14.86:1 |
| `muted` | `#ABBDD0` | 9.43:1 | 8.13:1 | 8.72:1 |
| `link` | `#8BC3FF` | 9.80:1 | 8.45:1 | 9.06:1 |
| `keyword` | `#F398C4` | 8.72:1 | 7.52:1 | 8.06:1 |
| `string` | `#A9D28B` | 10.61:1 | 9.14:1 | 9.80:1 |
| `comment` | `#ABBDD0` | 9.43:1 | 8.13:1 | 8.72:1 |
| `number` | `#E4B17D` | 9.40:1 | 8.10:1 | 8.69:1 |
| `function` | `#8BC3FF` | 9.80:1 | 8.45:1 | 9.06:1 |
| `type` | `#D3AAFF` | 9.47:1 | 8.16:1 | 8.75:1 |
| `math` | `#C6A2ED` | 8.46:1 | 7.29:1 | 7.82:1 |

| Non-text token | Hex | Canvas | Raised | Code |
| --- | --- | ---: | ---: | ---: |
| `border_control` | `#6F849B` | 4.71:1 | 4.06:1 | 4.35:1 |
| `border_focus` | `#8BC3FF` | 9.80:1 | 8.45:1 | 9.06:1 |
| `caret` | `#EBF2FA` | 16.08:1 | 13.86:1 | 14.86:1 |

`text.on_accent` appears **only on** the solid `accent.primary` fill: `#0E1623` / `#8BC3FF` = **9.80:1**. It is not used on canvas, raised, or code; for those surfaces use primary/muted/link above. The accent itself is 9.80:1 on canvas, 8.45:1 on raised, and 9.06:1 on code.

### Highlight text after alpha compositing

The `#RRGGBBAA` tokens are blended **in 8-bit sRGB** over each opaque surface first (`round(alpha × foreground + (1−alpha) × background)` per channel). The table gives the composited background and the **lowest contrast across primary, muted, link, keyword, string, comment, number, function, type, and math text** on that background. This is more stringent than checking only body text. Anti-aliasing can lower effective edge contrast, so a renderer review is still required before implementation.

| Theme | Highlight | Canvas composite / minimum text | Raised composite / minimum text | Code composite / minimum text | Limiting text |
| --- | --- | --- | --- | --- | --- |
| light | `selection` | `#D3DFEF` / **5.27:1** | `#CCDAEB` / **5.01:1** | `#C3D4E7` / **4.70:1** | `string` |
| light | `find_current` | `#F4E1C0` / **5.54:1** | `#EEDDBC` / **5.31:1** | `#E7D7BA` / **5.02:1** | `string` |
| dark | `selection` | `#27394F` / **5.49:1** | `#30445D` / **4.64:1** | `#2B3F58` / **5.01:1** | `math` |
| dark | `find_current` | `#43372E` / **5.37:1** | `#4B413A` / **4.63:1** | `#473D36` / **4.92:1** | `math` |

`find_match` is an existing alpha highlight and remains visually distinct with a continuous 1 px underline, while `find_current` gains a closed 1 px focus-colour outline. Thus current vs other matches differ in **shape** as well as colour; neither requires motion. If text appears on ordinary `find_match`, renderer review should confirm its contrast, too. The only intentionally translucent editor fills are the three pre-existing highlights.

These values correct the reported current muted text failures on raised/code (light 4.3/4.0, dark 4.0/3.7) and code comments (light 4.0, dark 3.7). Proposed raised/code muted values are light **6.90/6.42**, dark **8.13/8.72**; proposed code comment values are light **6.13**, dark **8.72**.

## Existing component styling

Spacing below is in px and restricted to the **4 px grid**; radii are **none 0**, **small 4**, **medium 8**. All boundaries and underlines are **1 px**. Colours use tokens above; no new screen or component is implied. “Focus” means a static, visible state. An overlay receives a max-width of the client width minus 16 px and a max-height of the client height minus 16 px, with internal scrolling, so a 480 × 360 client remains usable.

| Existing component | Radius | Padding / gap | Border | Focus or selected state | 480 × 360 behaviour |
| --- | --- | --- | --- | --- | --- |
| Canvas and content column | none | 16 outer / 0 column; 16 between blocks | 0; optional 1 subtle rule | Document focus uses steady caret | Column width ≤ min(820, client−32)=448; vertical scroll |
| Raw editing card | small | 12 / 8 | 1 subtle | Focused block gets 1 focus edge; selection uses editing.selection | Width ≤448; wrap or horizontally scroll code, never extend viewport |
| Caret | none | 0 / 0 | 1 caret line | Static; no blinking or transition | Visible in narrow column; scroll insertion point into view |
| Selection | none | 0 / 0 | 0; existing alpha fill | editing.selection behind selected glyphs | Clip to visible text; no extra overlay pass |
| Headings H1/H2 | none | 0 / 8 below | 1 subtle underline below H1/H2 | Keyboard focus follows caret; selected text uses selection | Wrap title within 448 width; underline follows width |
| Links | none | 0 / 0 | 1 accent underline | Keyboard focused link gets 1 focus outline; selected uses selection | Wrap long links; no horizontal page overflow |
| Inline code | small | 4 inline / 0 | 1 subtle | Selected text uses selection; code surface behind text | Wrap at safe opportunities; no clipping of glyphs |
| Code blocks | small | 12 / 8 between lines | 1 subtle | Focused raw code uses focus edge; selected text uses selection | Max width 448; horizontal scrolling inside code block |
| Blockquote bar | none | 8 left / 8 below | 1 subtle left bar | Caret/selection follow text; no bar animation | Wrap quote text inside column |
| List markers and checkboxes | small | 4 marker / 8 to text | checkbox: 1 control; markers: 0 | Checked state has solid accent fill + contrasting mark; focused checkbox 1 focus outline | Indent decreases within 448; keep marker visible when text wraps |
| Tables | small | 8 cell / 4 rows | 1 subtle grid | Focused cell gets 1 focus edge; selected text uses selection | Table scrolls horizontally inside column; no page overflow |
| Rules | none | 8 above and below / 0 | 1 subtle horizontal | No independent focus; selection stays on document | Rule spans available column width |
| Find/replace bar | medium | 8 / 8 controls | 1 control; focused input 1 focus | Selected result uses accent edge and find_current outline; other results underline | Width ≤464 with 8 margin; wrap controls and scroll if height >344 |
| Picker: Go to heading / Open recent | medium | 8 / 4 rows | 1 control; 1 focus on search | Selected row: solid accent fill + on_accent text; keyboard focus 1 focus edge | Width ≤464, height ≤344; list scrolls internally |
| Keyboard prompt (Linux) and scrim | medium prompt; none scrim | 12 prompt / 8; 0 scrim | prompt 1 control, focused control 1 focus; scrim 0 | Prompt focus uses focus; scrim is opaque surface.scrim, no translucency | Prompt ≤464 ×344 with internal scroll; scrim fills client |

The selected picker row uses an opaque accent fill and `on_accent` text, so no extra translucent fill is introduced. Underline and outlines are static 1 px geometry. For block-swap text styling, do not add text measurement or allocations to the keystroke path without measured evidence.

## Deprioritized for speed

- Bundled or brand fonts in the editor, runtime logo or SVG decoding, gradients in editor chrome, remote assets, shadows, translucency outside the existing highlights, splash screens, custom title bars, motion (including blinking caret), and new screens are excluded because they add startup or frame work without helping editing.

## Validation before implementation

Contrast calculations are for proposed values; GPUI rendering and platform color handling must be checked in the actual app. Review 480 × 360 on Windows and Linux, keyboard navigation, high DPI and both themes. If rendering code or icon resources change, follow the before/after startup and frame evidence in `docs/design/DESIGN_DIRECTION.md` and `AGENTS.md`; do not treat this documentation PR as a performance benchmark.
