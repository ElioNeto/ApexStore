# ApexStore v2.1.57 — Security Test Report

**Date:** 2026-05-22 16:53 UTC  
**Branch:** `test/stress-log-simulation`  
**Server:** HTTP API on port 9997, auth disabled (see #178)

---

## 1. Reconnaissance

| Test | Result | Verdict |
|------|--------|---------|
| Server header | `(none)` — no version disclosure | ✅ |
| Content-Type | `application/json` | ✅ |
| Endpoint discovery | All expected endpoints found (`/keys`, `/stats`, `/metrics`, `/admin/flush`, `/admin/compact`) | ✅ |
| CORS headers | Absent — no `Access-Control-*` returned | ⚠️ CORS not configured |
| HTTP methods | GET allowed on all, PUT/DELETE on `/keys/{key}`, POST on `/admin/*`, OPTIONS/HEAD/PATCH return 404 | ✅ |

## 2. Input Validation & Injection

| Test | Result | Verdict |
|------|--------|---------|
| Path traversal (7 variants) | All return `404` | ✅ Protected |
| NoSQL/key injection (9 variants) | All return `200` — key treated as literal string | ✅ No injection risk |
| Malformed JSON (10 variants) | `400` Bad Request | ✅ Properly rejected |
| 10KB key | `200` | ✅ Accepted |
| 1MB key | `200` timeout? (server busy) | ⚠️ Risk of large key DoS |
| Special characters in keys | Most work (`200`); slashes return `404` | ⚠️ Slash limitation |

## 3. Authentication

| Test | Result | Verdict |
|------|--------|---------|
| Token fuzzing (19 tokens) | All return `200` regardless of value | ❌ **Auth not wired** (#178) |
| Header injection (6 headers) | All `200` | ❌ Same issue |
| Missing Authorization header | `200` | ❌ No auth enforcement |

**All endpoints are publicly accessible.** The `bearer_validator` middleware exists but is never applied to the actix-web `App`.

## 4. Rate Limiting & DoS

| Test | Result | Verdict |
|------|--------|---------|
| 100 concurrent requests | 129ms, all successful | ⚠️ No rate limiting |
| 500 concurrent requests | 823ms, server became unresponsive after | ❌ **DoS vulnerability** (#185) |
| 500KB PUT payload | `400` — rejected | ✅ |
| 1MB+ PUT payload | `400` — rejected | ✅ Payload limit works |

## 5. Information Disclosure

| Test | Result | Verdict |
|------|--------|---------|
| Server version header | Not disclosed | ✅ |
| X-Powered-By header | Not present | ✅ |
| Directory listing | None — all return `404` | ✅ |
| Error messages | No stack traces or internal paths leaked | ✅ |
| Stats endpoint | Exposes key count, table count, sizes (expected) | ✅ |
| Metrics endpoint | Exposes operation counters (expected for Prometheus) | ✅ |

## 6. Dependency Vulnerabilities (cargo audit)

| Advisory | Crate | Version | Severity | Status |
|----------|-------|---------|----------|--------|
| RUSTSEC-2025-0141 | **bincode** | 1.3.3 | UNMAINTAINED | ❌ **Needs replacement** (#187) |
| RUSTSEC-2024-0436 | paste | 1.0.15 | UNMAINTAINED | ⚠️ Transitive via ratatui |
| RUSTSEC-2026-0002 | lru | 0.12.5 | UNSOUND | ⚠️ Transitive via ratatui |

## 7. Static Analysis (Code Quality)

| Pattern | Count | Locations |
|---------|-------|-----------|
| `unwrap()` in production | 2 | `engine/mod.rs:170`, `engine/mod.rs:1594` |
| `expect()` in production | 4 | `engine/mod.rs:167,1581`, `version_set.rs:32`, `cache.rs:41` |
| `panic!()` in production | 1 | `reader.rs:529` (under `#[cfg(test)]` — safe) |
| `unsafe` blocks | 0 | ✅ |
| Hardcoded secrets | 0 | ✅ |

**6 unwrap/expect calls** in production code can crash the engine (#186).

## 8. Transport Security

| Issue | Severity |
|-------|----------|
| HTTP only, no HTTPS | 🔴 **High** — MITM risk |
| No TLS configuration option | 🟡 Medium |
| Recommendation | Deploy behind TLS-terminating reverse proxy (nginx, caddy) |

## 9. Summary

### Critical Issues (0)
None found in the test scope.

### High Severity (3)
| # | Issue |
|---|-------|
| #182 | No SIGTERM handler — data loss on shutdown |
| #185 | No rate limiting — server crashes under 500 concurrent connections |
| — | HTTP-only transport (no TLS) |

### Medium Severity (5)
| # | Issue |
|---|-------|
| #178 | Auth middleware never wired — all endpoints public |
| #180 | Cold SSTable reads always miss |
| #183 | No cargo audit in CI |
| #186 | 6 unwrap/expect calls in production code |
| #187 | bincode dependency is UNMAINTAINED |

### Low Severity (1)
| # | Issue |
|---|-------|
| #179 | CLI has no token management commands |

### Protected Areas ✅
- Path traversal attacks (all 7 variants → 404)
- SQL/NoSQL injection (all 9 variants → 200 safe)
- Malformed JSON (→ 400)
- Large payloads >500KB (→ 400)
- Directory listing (→ 404)
- Server version disclosure (none)
- Stack trace leakage (none)
- Unsafe Rust blocks (zero)
- Hardcoded secrets (zero)
