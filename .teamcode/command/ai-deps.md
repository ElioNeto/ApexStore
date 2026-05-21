---
description: "Check for available Rust dependency updates (minor/patch only)"
---

Check for available minor/patch updates to Rust dependencies in Cargo.toml.

Your job is to look into the Rust dependencies listed in `Cargo.toml`, figure out if they have versions that can be upgraded (minor or patch versions ONLY, no major changes).

I want a report of every dependency and the version that can be upgraded to.
What would be even better is if you can give me a brief summary of the changes for each dep and a link to the changelog for each dependency, or at least some reference info so I can see what bugs were fixed or new features were added.

Use `cargo outdated` if available, or check crates.io for each dependency.

DO NOT upgrade the dependencies yet, just make a list of all dependencies and their versions that can be upgraded to minor or patch versions only.

Write up your findings to `rust-deps-updates.md`.
