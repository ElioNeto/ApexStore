---
description: "Start development servers/binaries for ApexStore"
---

Start the ApexStore development targets.

## Options

- `server` — Start the HTTP API server:
  ```bash
  cargo run --bin apexstore-server
  ```
  Starts the actix-web REST API on the configured port (default 8080).

- `cli` — Start the CLI:
  ```bash
  cargo run --bin apexstore-cli -- <args>
  ```
  Runs the command-line interface for database operations.

- `tui` — Start the TUI:
  ```bash
  cargo run --bin apexstore-tui
  ```
  Opens the interactive terminal UI built with ratatui + crossterm.

- `release` — Run a release build binary:
  ```bash
  cargo run --release --bin apexstore-server
  ```

$ARGUMENTS

If no target is specified, defaults to `apexstore-server`.
