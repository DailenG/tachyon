# Setting up the project

Here's a step-by-step guide to get everything running locally.

1. **Install the toolchain**
   - On Windows, use the installer from the official site.
   - On Linux:
     ```bash
     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

     rustup component add clippy rustfmt
     ```
   - On macOS, Homebrew works too: `brew install rustup`.
2. **Clone the repository**

   ```sh
   git clone https://github.com/example/project.git
   cd project
   ```

   > **Note:** the repository uses Git LFS for assets.

3. **Build it**
   1. Debug: `cargo build`
   2. Release: `cargo build --release`
      - This enables LTO, so it takes longer.
      - The binary ends up in `target/release/`.

- [x] Toolchain installed
- [ ] Tests passing
  - [ ] Unit tests
  - [ ] Integration tests

That's it! Let me know if you hit any errors.
