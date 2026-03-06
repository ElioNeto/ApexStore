# Contributing to ApexStore

First off, thank you for considering contributing to ApexStore! 🎉

This document provides guidelines and instructions for contributing to the project. Following these guidelines helps maintain code quality and makes the review process smoother.

## 📜 Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Coding Standards](#coding-standards)
- [Testing Guidelines](#testing-guidelines)
- [Commit Messages](#commit-messages)
- [Pull Request Process](#pull-request-process)
- [Project Structure](#project-structure)
- [Areas for Contribution](#areas-for-contribution)

---

## 🤝 Code of Conduct

### Our Pledge

We are committed to providing a welcoming and inspiring community for all. We pledge to:

- Be respectful and inclusive
- Accept constructive criticism gracefully
- Focus on what is best for the community
- Show empathy towards other community members

### Expected Behavior

- Use welcoming and inclusive language
- Be respectful of differing viewpoints and experiences
- Gracefully accept constructive criticism
- Focus on what is best for the community

### Unacceptable Behavior

- Trolling, insulting/derogatory comments, and personal or political attacks
- Public or private harassment
- Publishing others' private information without explicit permission
- Other conduct which could reasonably be considered inappropriate

---

## 🚀 Getting Started

### Prerequisites

1. **Rust Toolchain** (1.70 or later)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Git**
   ```bash
   # Ubuntu/Debian
   sudo apt-get install git
   
   # macOS
   brew install git
   ```

3. **Code Editor** (Recommended: VS Code with rust-analyzer)
   - Install [VS Code](https://code.visualstudio.com/)
   - Install [rust-analyzer extension](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

### Initial Setup

1. **Fork the Repository**
   - Visit [https://github.com/ElioNeto/ApexStore](https://github.com/ElioNeto/ApexStore)
   - Click the "Fork" button in the top-right corner

2. **Clone Your Fork**
   ```bash
   git clone https://github.com/YOUR_USERNAME/ApexStore.git
   cd ApexStore
   ```

3. **Add Upstream Remote**
   ```bash
   git remote add upstream https://github.com/ElioNeto/ApexStore.git
   ```

4. **Install Dependencies**
   ```bash
   cargo build
   ```

5. **Run Tests**
   ```bash
   cargo test
   ```

For detailed setup instructions, see [SETUP.md](SETUP.md).

---

## 🔄 Development Workflow

### Automated Workflow Overview

ApexStore uses GitHub Actions to automate the development workflow:

- **Feature/Fix branches** → Auto-create PR to `develop` + run tests
- **Develop** → Auto-create release PR to `main`
- **Main** → Create release tag + close related issues

See [WORKFLOWS.md](WORKFLOWS.md) for complete documentation.

### 1. Create a Feature Branch

```bash
# Update your fork
git checkout main
git pull upstream main

# Create a new branch from main
git checkout -b feature/your-feature-name
```

**Branch Naming Conventions**:
- `feature/` - New features (e.g., `feature/compaction-strategy`)
- `fix/` - Bug fixes (e.g., `fix/wal-corruption`)
- `docs/` - Documentation changes (e.g., `docs/api-guide`)
- `refactor/` - Code refactoring (e.g., `refactor/codec-interface`)
- `test/` - Test additions/improvements (e.g., `test/integration-suite`)
- `perf/` - Performance improvements (e.g., `perf/bloom-filter-optimization`)

### 2. Make Your Changes

```bash
# Make changes to the code
vim src/core/engine.rs

# Test your changes
cargo test --all-features

# Format code
cargo fmt

# Check for issues
cargo clippy --all-features -- -D warnings
```

### 3. Commit Your Changes

```bash
git add .
git commit -m "feat: add compaction strategy interface (#55)"
```

**Important**: Reference issues in commit messages using `#issue_number` for automatic tracking.

See [Commit Messages](#commit-messages) for formatting guidelines.

### 4. Push to Your Fork

```bash
git push origin feature/your-feature-name
```

**What Happens Next:**
- ✅ GitHub Actions automatically runs tests
- ✅ Auto-creates PR to `develop` branch
- ✅ Adds comment to referenced issues (if any)
- ✅ Runs Clippy and format checks

### 5. Pull Request Review

Once the automated PR is created:

1. Review the PR description
2. Wait for CI checks to pass
3. Address any reviewer feedback
4. PR will be merged to `develop` once approved

**Note**: You don't need to manually create PRs - the workflow handles this automatically!

---

## 📝 Coding Standards

### Rust Style Guide

We follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) and [Rust Style Guide](https://doc.rust-lang.org/1.0.0/style/).

**Key Principles**:

1. **Use `cargo fmt`** - All code must be formatted
   ```bash
   cargo fmt --all
   ```

2. **Pass `cargo clippy`** - Zero warnings policy
   ```bash
   cargo clippy --all-features -- -D warnings
   ```

3. **Write Documentation** - Public APIs must have doc comments
   ```rust
   /// Retrieves a value from the store by key.
   ///
   /// # Arguments
   ///
   /// * `key` - The key to look up
   ///
   /// # Returns
   ///
   /// * `Ok(Some(value))` - Key found
   /// * `Ok(None)` - Key not found
   /// * `Err(e)` - Error occurred
   ///
   /// # Example
   ///
   /// ```
   /// let value = engine.get(b"user:123")?;
   /// ```
   pub fn get(&self, key: &[u8]) -> Result<Option<String>> {
       // Implementation
   }
   ```

### SOLID Principles

This project follows SOLID principles:

- **Single Responsibility**: Each module/struct has one clear purpose
- **Open/Closed**: Extend behavior through traits, not modification
- **Liskov Substitution**: Implementations must be interchangeable
- **Interface Segregation**: Small, focused traits
- **Dependency Inversion**: Depend on abstractions, not concretions

**Example**:
```rust
// ✅ Good - depends on trait
pub struct LsmEngine<W: WriteAheadLog> {
    wal: W,
}

// ❌ Bad - depends on concrete type
pub struct LsmEngine {
    wal: FileBasedWal,
}
```

### Error Handling

1. **Use `Result<T, LsmError>`** for fallible operations
   ```rust
   pub fn put(&mut self, key: &[u8], value: &str) -> Result<()> {
       self.wal.append(key, value)?;
       self.memtable.insert(key, value);
       Ok(())
   }
   ```

2. **Provide Context** with error types
   ```rust
   use thiserror::Error;

   #[derive(Error, Debug)]
   pub enum LsmError {
       #[error("WAL corruption at offset {0}")]
       WalCorruption(u64),
       
       #[error("Key too large: {size} bytes (max: {max})")]
       KeyTooLarge { size: usize, max: usize },
   }
   ```

3. **Don't Panic** in library code (use `Result` instead)

### Performance Considerations

1. **Minimize Allocations**
   ```rust
   // ✅ Good - reuse buffer
   let mut buffer = Vec::with_capacity(1024);
   for item in items {
       buffer.clear();
       serialize_into(&mut buffer, item)?;
   }

   // ❌ Bad - allocate each iteration
   for item in items {
       let buffer = serialize(item)?;
   }
   ```

2. **Use Appropriate Data Structures**
   - `BTreeMap` for sorted data
   - `HashMap` for fast lookups
   - `Vec` for sequential access

3. **Benchmark Changes**
   ```bash
   cargo bench
   ```

---

## 🧪 Testing Guidelines

### Test Types

1. **Unit Tests** - Test individual functions/modules
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_memtable_insert() {
           let mut memtable = MemTable::new();
           memtable.insert(b"key", "value");
           assert_eq!(memtable.get(b"key"), Some("value".to_string()));
       }
   }
   ```

2. **Integration Tests** - Test component interactions
   ```rust
   // tests/integration_test.rs
   #[test]
   fn test_engine_recovery() {
       let config = LsmConfig::default();
       let mut engine = LsmEngine::new(config).unwrap();
       
       engine.put(b"key", "value").unwrap();
       drop(engine);
       
       let engine = LsmEngine::new(config).unwrap();
       assert_eq!(engine.get(b"key").unwrap(), Some("value".to_string()));
   }
   ```

3. **Property Tests** - Test invariants (optional, using `proptest`)

### Test Requirements

- **All new code must have tests**
- **Tests must pass on all platforms**
- **Test coverage should increase, not decrease**
- **Use descriptive test names**
  ```rust
  #[test]
  fn test_get_returns_none_for_nonexistent_key() { /* ... */ }
  ```

### Running Tests

```bash
# Run all tests
cargo test --all-features

# Run specific test
cargo test test_memtable_insert

# Run with output
cargo test -- --nocapture

# Run integration tests only
cargo test --test '*'

# Run with coverage (requires tarpaulin)
cargo tarpaulin --out Html
```

---

## 📝 Commit Messages

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification.

### Format

```
<type>(<scope>): <subject> (#issue)

<body>

<footer>
```

### Types

- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation changes
- `style` - Code style changes (formatting, etc.)
- `refactor` - Code refactoring
- `perf` - Performance improvements
- `test` - Test additions/modifications
- `chore` - Build process, dependencies, tooling
- `ci` - CI/CD changes

### Examples

**Simple commit**:
```
feat: add bloom filter to SSTable reader
```

**With scope and issue**:
```
fix(wal): prevent corruption on unclean shutdown (#42)
```

**With body**:
```
feat(compaction): implement leveled compaction strategy (#47)

Adds a new LeveledCompaction struct that implements the Compaction
trait. This strategy reduces read amplification by maintaining
sorted levels with exponentially increasing sizes.

Closes #47
```

**Breaking change**:
```
feat(api)!: change SSTable format to V2

BREAKING CHANGE: SSTable V2 is incompatible with V1.
Migration tool will be provided in v1.4.
```

**Referencing Issues**:
- `#123` - Reference issue
- `fixes #123`, `closes #123` - Will auto-close issue when PR merges
- `resolves #123` - Alternative close syntax

---

## 🔍 Pull Request Process

### Before Pushing

- [ ] Code compiles without errors
- [ ] All tests pass (`cargo test --all-features`)
- [ ] No clippy warnings (`cargo clippy --all-features -- -D warnings`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] Documentation is updated (if applicable)
- [ ] Tests are added for new functionality

### Automated PR Creation

When you push to a `feature/*` or `fix/*` branch:

1. ✅ **Tests run automatically** (CI/CD)
2. ✅ **PR is auto-created** to `develop` branch
3. ✅ **Issues are commented** (if referenced in commits)
4. ✅ **Checks must pass** before merge

### Manual Steps (If Needed)

If you need to create a PR manually:

1. Go to your fork on GitHub
2. Click "New Pull Request"
3. Select `develop` as the base branch
4. Fill out the PR template
5. Submit the PR

### PR Template

When creating a PR manually, use this template:

```markdown
## Description

Brief description of changes.

## Type of Change

- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Related Issues

Closes #123
Related to #456

## Testing

Describe how you tested your changes:
- [ ] Unit tests added
- [ ] Integration tests added
- [ ] Manual testing performed

## Checklist

- [ ] Code compiles
- [ ] Tests pass
- [ ] Clippy checks pass
- [ ] Code is formatted
- [ ] Documentation updated

## Screenshots (if applicable)

## Additional Notes
```

### Review Process

1. **Automated Checks** - CI runs tests and linters
2. **Code Review** - Maintainer reviews code
3. **Feedback** - Address review comments
4. **Approval** - Maintainer approves PR
5. **Merge** - Squash and merge to `develop`

### Review Timeline

- **Simple PRs**: 1-3 days
- **Complex PRs**: 3-7 days
- **Breaking Changes**: 7-14 days

---

## 📁 Project Structure

```
ApexStore/
├── src/
│   ├── core/              # Core domain logic
│   │   ├── engine.rs      # LSM engine orchestration
│   │   ├── memtable.rs    # In-memory storage
│   │   └── log_record.rs  # Data model
│   ├── storage/           # Persistence layer
│   │   ├── wal.rs         # Write-ahead log
│   │   ├── sstable.rs     # SSTable reader
│   │   └── builder.rs     # SSTable writer
│   ├── infra/             # Infrastructure
│   │   ├── codec.rs       # Serialization
│   │   ├── error.rs       # Error types
│   │   └── config.rs      # Configuration
│   ├── api/               # HTTP API (feature-gated)
│   ├── cli/               # CLI interface
│   └── features/          # Feature flags
├── tests/                 # Integration tests
├── benches/               # Benchmarks
└── docs/                  # Documentation
```

### Module Guidelines

- **`core/`** - Domain logic, no external dependencies
- **`storage/`** - File I/O, persistence
- **`infra/`** - Cross-cutting concerns
- **`api/`** - External interfaces (feature-gated)

---

## 🎯 Areas for Contribution

### High Priority

1. **CI/CD Testing Pipeline** (#55)
   - Difficulty: Easy
   - Impact: High
   - Skills: GitHub Actions, YAML

2. **Compaction Implementation** (#47)
   - Difficulty: Hard
   - Impact: High
   - Skills: Rust, algorithms, file I/O

3. **Efficient Iterators** (#21, #22, #23)
   - Difficulty: Medium
   - Impact: High
   - Skills: Rust, data structures

### Medium Priority

4. **Benchmarking Suite** (#48)
   - Difficulty: Easy
   - Impact: Medium
   - Skills: Rust, criterion

5. **CLI Command Equalization** (#65)
   - Difficulty: Medium
   - Impact: Medium
   - Skills: Rust, CLI design

6. **Checksums & Integrity** (#25)
   - Difficulty: Easy
   - Impact: High
   - Skills: Rust, CRC32

### Good First Issues

7. **Add More Tests**
   - Difficulty: Easy
   - Impact: Medium
   - Skills: Rust, testing

8. **Binary Search Optimization** (#37)
   - Difficulty: Easy
   - Impact: Medium
   - Skills: Rust, algorithms

9. **Documentation Improvements**
   - Difficulty: Easy
   - Impact: Medium
   - Skills: Technical writing

### Advanced Topics

10. **Replication Support**
    - Difficulty: Very Hard
    - Impact: Very High
    - Skills: Distributed systems, Raft

11. **Snapshot Isolation**
    - Difficulty: Hard
    - Impact: High
    - Skills: Concurrency, MVCC

---

## ❓ Questions?

- **Issues**: [GitHub Issues](https://github.com/ElioNeto/ApexStore/issues)
- **Discussions**: [GitHub Discussions](https://github.com/ElioNeto/ApexStore/discussions)
- **Documentation**: [docs/](docs/)
- **Email**: netoo.elio@hotmail.com

---

## 🚀 Ready to Contribute?

1. Find an issue or create one
2. Comment that you're working on it
3. Fork the repo and create a branch from `main`
4. Make your changes with tests
5. Push to your fork (automated PR will be created)
6. Wait for review and address feedback

**Thank you for contributing!** 🎉

---

*Last updated: March 2026*
