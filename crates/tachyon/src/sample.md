# Tachyon

A scratchpad for **Markdown**, rendered _raw_ until the block editor lands.

## What this window shows

- Text is shaped and drawn by GPUI on the GPU.
- No WebView, no Chromium, no Electron.
- Launch `tachyon notes.md` to open a file, or `tachyon --paste` for the clipboard.

```rust
fn main() {
    println!("hello from a fenced code block");
}
```

| Milestone | Target |
|-----------|--------|
| First frame | < 50 ms |
| Keystroke reparse | < 0.5 ms |

> Blockquotes, [links](https://example.com) and `inline code` show their syntax here.
