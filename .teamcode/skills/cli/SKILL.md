---
name: cli
description: Work with the CLI and TUI applications. Use when modifying CLI commands (clap), the TUI (ratatui), argument parsing, or command-line workflows.
---

# CLI & TUI

ApexStore has two terminal interfaces:
- **CLI** (`apexstore-cli`) — command-line tool using clap for argument parsing
- **TUI** (`apexstore-tui`) — interactive terminal UI using ratatui + crossterm

## Architecture

```
src/bin/
├── cli.rs              # CLI entry point (clap argument parsing)
└── tui.rs              # TUI entry point (ratatui + crossterm)

src/
├── cli/                # CLI command implementations
│   ├── commands.rs     # Individual subcommands
│   └── format.rs       # Output formatting
└── tui/                # TUI components
    ├── app.rs          # Main TUI app
    ├── ui.rs           # UI rendering (ratatui widgets)
    ├── event.rs        # Event handling (crossterm)
    ├── components/     # UI components (panels, lists, etc.)
    └── state.rs        # Application state
```

## Key Commands

| Command | Description |
|---------|-------------|
| `get <cf> <key>` | Get value by key |
| `put <cf> <key> <value>` | Put key-value pair |
| `delete <cf> <key>` | Delete key |
| `scan <cf> [--prefix] [--range]` | Scan keys |
| `cf create <name>` | Create column family |
| `cf list` | List column families |
| `cf delete <name>` | Delete column family |
| `flush` | Flush memtable to SSTable |
| `compact` | Trigger compaction |
| `stats` | Show database statistics |

### Development

```bash
# Build CLI
cargo build --bin apexstore-cli

# Build TUI
cargo build --bin apexstore-tui

# Run CLI
cargo run --bin apexstore-cli -- get mycf mykey

# Run TUI
cargo run --bin apexstore-tui
```

## TUI Features

- Built with ratatui widgets
- crossterm for terminal I/O and event handling
- Keyboard-driven navigation
- Real-time stats dashboard
- Column family browser
- Key-value editor
