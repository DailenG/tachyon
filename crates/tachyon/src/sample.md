# Tachyon

A scratchpad for **Markdown**. Click any block, or move into it with the arrow keys, to edit its
raw source; every other block stays rendered.

## Try it

- Paste an LLM answer with `Ctrl+V`
- Type `## ` at the start of a line to make a heading
- Undo with `Ctrl+Z`, redo with `Ctrl+Shift+Z`
- [x] Rendered with GPUI on the GPU
- [ ] No WebView, no Chromium, no Electron

```rust
fn main() {
    println!("hello from a fenced code block");
}
```

| Milestone | Target |
|:----------|-------:|
| First frame | < 50 ms |
| Keystroke reparse | < 0.5 ms |

> Blockquotes, [links](https://example.com), `inline code`, ~~strikethrough~~ and $e^{i\pi}$ math.
