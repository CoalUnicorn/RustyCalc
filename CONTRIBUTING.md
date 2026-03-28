# Contributing to RustyCalc

Whether you are a seasoned developer or a rookie, welcome to RustyCalc! 

🎉 We appreciate your interest in contributing to our project.

Before starting any work it is best if you get in touch to make sure your work is relevant.

---

## 🛠 Changes to the main repo

If you are comfortable working with GitHub and Git, the following steps should be straightforward. For more general information visit [GitHub Docs](https://docs.github.com/en) and [Git Documentation](https://git-scm.com/doc). 

1. **Fork the repository**
   Start by forking the repository to your own GitHub account. You can do this by clicking the "Fork" button on the top right of the repository page.

2. **Clone the original repository**
   Clone the original repository to your local machine:
   ```bash
   git clone https://github.com/CoalUnicorn/RustyCalc.git
   cd RustyCalc
   ```

3. **Add your fork as a remote**
    Add your forked repository as a remote named `fork`:

    ```bash
    git remote add fork https://github.com/<your-username>/RustyCalc.git
    ```

4. **Create a new branch**
    Always create a new branch for your changes to keep your work isolated:

    ```bash
    git checkout -b your-feature-name
    ```

5. **Make changes**
    Implement your changes, improvements, or bug fixes. Make sure to follow the coding style and project-specific guidelines below.

6. **Commit your changes**
    Write clear and concise commit messages:

    ```bash
    git add .
    git commit -m "Brief description of your changes"
    ```

7. **Push to your fork**
    Push your branch to your forked repository:
    ```bash
    git push fork your-feature-name
    ```

8. **Create a Pull Request (PR)**
   Follow the steps on the terminal or go to the original RustyCalc repository, and click on "New Pull Request."
   Ensure your PR has a clear title and description explaining the purpose of your changes.

### Keeping Your Fork Up to Date

Always start from the main branch in a clean state. To keep your fork synchronized with the original repository:

```bash
# Switch to main branch
git checkout main

# Fetch latest changes from the original repository
git pull origin main

# Push updates to your fork
git push fork main

# Create new feature branch from updated main
git checkout -b your-new-feature-name
```

You should make sure that your changes are properly tested before submitting.

---

## 📋 Code Style and Guidelines

RustyCalc follows strong Rust design principles. Please review [docs/rust-style-guide.md](docs/rust-style-guide.md) for the patterns and conventions used throughout the codebase.

### Key Principles:
- **Domain types**: Use new types instead of bare `String`/primitives
- **Enums for state**: Use enums for closed sets and state machines instead of booleans
- **Parse don't validate**: Parse at boundaries, use typed values internally  
- **Exhaustive matching**: No wildcard `_ =>` arms on owned enums
- **Borrow by default**: Use `&str`, `&[T]` unless ownership is needed

### Performance Guidelines:
- **Critical**: Read [docs/performance-evaluation.md](docs/performance-evaluation.md) to understand the unified `mutate()` function and avoid double evaluation issues

### Adding Features:
- **Keyboard shortcuts**: See [docs/adding-actions.md](docs/adding-actions.md)
- **New components**: See [docs/building-components.md](docs/building-components.md)  
- **Leptos patterns**: See [docs/leptos-patterns.md](docs/leptos-patterns.md)

---

## 🧪 Development Setup

1. **Install Rust and WebAssembly target:**
```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

2. **Run the development server:**
```bash
trunk serve
# Visit http://localhost:8080/RustyCalc/
```

3. **Run tests:**
```bash
wasm-pack test --headless --firefox
```

4. **Build for production:**
```bash
trunk build --release
```

5. **Desktop build (optional):**
```bash
cargo tauri dev
```

---

## 🧪 Writing Tests

RustyCalc uses `wasm-pack test` for browser-based testing. See [docs/testing-guide.md](docs/testing-guide.md) for comprehensive testing guidelines.

### Quick Start:
```rust
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn test_my_feature() {
    // Your test logic here
    assert_eq!(expected, actual);
}
```

### Test Categories:
- **Unit tests**: Test individual functions and components
- **Integration tests**: Test action dispatch and state management
- **UI tests**: Test component rendering and interaction
- **Performance tests**: Verify no double evaluation issues

---

## 📂 Project Structure

Key directories for contributors:

- `src/model/` — Domain types and data structures
- `src/state.rs` — UI state management with Leptos signals
- `src/components/` — Leptos components for UI
- `src/input/` — Action system for keyboard/mouse input
- `src/canvas/` — Canvas 2D rendering
- `src/storage.rs` — LocalStorage persistence
- `docs/` — Documentation and guides
- `src-tauri/` — Optional desktop shell

## Submitting Changes

Before submitting your PR:

1. **Code Quality**: Ensure your code follows the style guide patterns
2. **Tests**: Add tests for new functionality  
3. **Documentation**: Update docs if needed
4. **Verification**: Ensure `cargo check` and `wasm-pack test` pass
5. **Description**: Submit a PR with a clear title and description

### PR Checklist:
- [ ] Code follows Rust style guidelines
- [ ] Tests added for new functionality
- [ ] Documentation updated (if applicable)
- [ ] `cargo check` passes without errors
- [ ] `wasm-pack test` passes
- [ ] No performance regressions (double evaluation)
- [ ] Clear commit messages and PR description

---

## 🤝 Community and Support

Feel free to reach out if you have questions or need help:

- **Issues**: Open a GitHub issue to report bugs or discuss features
- **Discussions**: Use GitHub Discussions for questions and ideas  
- **Email**: Contact maintainers for general inquiries

### Ways to Contribute:
- **Code**: Features, bug fixes, performance improvements
- **Documentation**: Guides, examples, API docs
- **Testing**: Bug reports, test cases, QA
- **Ideas**: Feature suggestions, UX improvements
- **Design**: UI/UX, themes, icons

Note that not all contributors need to be coding. Testing, bug reports, documentation improvements, and ideas are all valuable contributions.

Thank you for your contributions! 💪 Together, we can make RustyCalc even better.
