# Contributing to Ralph CLI

We welcome contributions to the Ralph CLI! Here's how you can get started.

## How to Build

To build the Ralph CLI from source, you'll need Rust installed. If you don't have Rust, you can install it using `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Once Rust is installed, navigate to the project root and run:

```bash
cargo build --release
```

This will compile the `ralph` executable in `target/release/`.

## How to Run Tests

To run the project's tests, use the following command from the project root:

```bash
cargo test
```

## How to Add a New Agent

Ralph supports different agents (e.g., Claude, Gemini). To add a new agent:

1.  **Create a new module:** In the `src/agents/` directory, create a new Rust file (e.g., `src/agents/my_new_agent.rs`).
2.  **Implement the Agent trait:** Your new agent struct must implement the `Agent` trait defined in `src/agents/mod.rs`. This trait requires methods for parsing PRDs and handling agent-specific logic.
3.  **Register the agent:**
    *   Add your new module to `src/agents/mod.rs` using `mod my_new_agent;`.
    *   Integrate your agent into the agent selection logic (e.g., in `src/parser/mod.rs` or `src/cli.rs`) so it can be invoked via CLI flags or configuration.

## How to Submit Pull Requests

1.  **Fork the repository:** Fork the Ralph CLI repository on GitHub.
2.  **Clone your fork:**
    ```bash
    git clone https://github.com/your-username/ralph.git
    cd ralph
    ```
3.  **Create a new branch:**
    ```bash
    git checkout -b feature/your-feature-name
    ```
4.  **Make your changes:** Implement your feature or bug fix.
5.  **Run tests:** Ensure all existing tests pass (`cargo test`) and add new tests for your changes if applicable.
6.  **Format and lint:**
    ```bash
    cargo fmt
    cargo clippy -- -D warnings
    ```
7.  **Commit your changes:** Write clear and concise commit messages.
8.  **Push to your fork:**
    ```bash
    git push origin feature/your-feature-name
    ```
9.  **Open a Pull Request:** Go to the original Ralph CLI repository on GitHub and open a pull request from your fork and branch. Describe your changes clearly.