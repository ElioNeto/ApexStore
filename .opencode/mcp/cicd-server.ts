#!/usr/bin/env npx tsx
/**
 * CI/CD MCP Server
 *
 * Exposes tools for running CI pipeline and checking TODOs locally.
 * Protocol: JSON-RPC 2.0 over stdin/stdout (MCP standard).
 *
 * Usage:
 *   npx tsx .opencode/mcp/cicd-server.ts
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { execSync } from "node:child_process";

// ── MCP Protocol helpers ────────────────────────────────────────────────────

type JsonRpcRequest = {
  jsonrpc: "2.0";
  id: number | string;
  method: string;
  params?: Record<string, unknown>;
};

type JsonRpcResponse = {
  jsonrpc: "2.0";
  id: number | string | null;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
};

function send(msg: JsonRpcResponse): void {
  const line = JSON.stringify(msg);
  process.stdout.write(line + "\n");
}

// ── Line-buffered stdin reader ───────────────────────────────────────────────
//
// The original implementation used a one-shot listener that was removed after
// the first line.  When the client piped multiple messages in a single chunk
// (e.g. initialize + notifications/initialized + tools/list), only the first
// was processed and the rest were silently dropped — causing "failed to get
// tools" because the tools/list request never arrived.
//
// This version maintains a persistent buffer and resolves complete lines one
// at a time via a queue of pending promises.

let stdinBuffer = "";
let stdinResolvers: Array<(line: string | null) => void> = [];

function flushLines(): void {
  while (stdinResolvers.length > 0) {
    const nl = stdinBuffer.indexOf("\n");
    if (nl === -1) break; // no complete line yet
    const line = stdinBuffer.slice(0, nl).trim();
    stdinBuffer = stdinBuffer.slice(nl + 1);
    const resolve = stdinResolvers.shift()!;
    resolve(line || null); // skip empty lines
  }
}

process.stdin.on("data", (chunk: Buffer) => {
  stdinBuffer += chunk.toString();
  flushLines();
});

process.stdin.on("end", () => {
  // If there's remaining content, flush it as the last line
  if (stdinBuffer.trim()) {
    for (const resolve of stdinResolvers) {
      resolve(stdinBuffer.trim());
    }
    stdinResolvers = [];
    stdinBuffer = "";
  }
  // Resolve any remaining pending promises with null (end-of-stream)
  for (const resolve of stdinResolvers) {
    resolve(null);
  }
  stdinResolvers = [];
});

function readLine(): Promise<string | null> {
  return new Promise((resolve) => {
    stdinResolvers.push(resolve);
    flushLines(); // in case data arrived before the promise was set up
  });
}

// ── Tool implementations ────────────────────────────────────────────────────

const WORKSPACE = process.cwd();

function runCommand(cmd: string, args: string[]): { stdout: string; stderr: string; exitCode: number } {
  try {
    const result = execSync(`${cmd} ${args.join(" ")}`, {
      cwd: WORKSPACE,
      encoding: "utf-8",
      maxBuffer: 10 * 1024 * 1024, // 10MB
      stdio: ["pipe", "pipe", "pipe"],
    });
    return { stdout: result, stderr: "", exitCode: 0 };
  } catch (err: any) {
    return {
      stdout: err.stdout || "",
      stderr: err.stderr || err.message || "",
      exitCode: err.status ?? 1,
    };
  }
}

async function handleToolCall(name: string, args: Record<string, unknown>): Promise<unknown> {
  switch (name) {
    // ── run_ci_pipeline ─────────────────────────────────────────────
    case "run_ci_pipeline": {
      const workflowFile = (args.workflow as string) || ".github/workflows/ci.yml";
      const scriptPath = path.join(WORKSPACE, "scripts", "workflow-agent.ts");

      if (!fs.existsSync(scriptPath)) {
        return { error: `workflow-agent.ts not found at ${scriptPath}` };
      }

      const result = runCommand("npx", ["tsx", scriptPath, workflowFile]);
      return {
        workflow: workflowFile,
        exitCode: result.exitCode,
        stdout: result.stdout.slice(0, 50000), // limit output
        stderr: result.stderr.slice(0, 10000),
        status: result.exitCode === 0 ? "success" : "failed",
      };
    }

    // ── check_todos ─────────────────────────────────────────────────
    case "check_todos": {
      const stateFile = (args.state_file as string) || ".task-state.json";
      const statePath = path.join(WORKSPACE, stateFile);
      const scriptPath = path.join(WORKSPACE, "scripts", "check-todos.ts");

      if (!fs.existsSync(statePath)) {
        return { error: `State file not found at ${statePath}`, ok: false };
      }
      if (!fs.existsSync(scriptPath)) {
        return { error: `check-todos.ts not found at ${scriptPath}` };
      }

      const result = runCommand("npx", ["tsx", scriptPath, stateFile]);
      let parsed;
      try {
        parsed = JSON.parse(result.stdout);
      } catch {
        parsed = { ok: false, error: "Failed to parse output", raw: result.stdout.slice(0, 2000) };
      }
      return { ...parsed, exitCode: result.exitCode, stderr: result.stderr.slice(0, 2000) };
    }

    // ── run_tests ────────────────────────────────────────────────────
    case "run_tests": {
      const argsList = (args.args as string) || "--all-features --workspace";
      const result = runCommand("cargo", ["test", ...argsList.split(/\s+/)]);
      const lastLines = result.stdout
        .split("\n")
        .slice(-30)
        .join("\n");
      return {
        exitCode: result.exitCode,
        status: result.exitCode === 0 ? "passed" : "failed",
        summary: lastLines,
        stderr: result.stderr.slice(0, 5000),
      };
    }

    // ── run_clippy ───────────────────────────────────────────────────
    case "run_clippy": {
      const argsList = (args.args as string) || "--all-targets --all-features";
      const result = runCommand("cargo", ["clippy", ...argsList.split(/\s+/)]);
      // clippy outputs to stderr
      const output = result.stderr || result.stdout;
      const lastLines = output.split("\n").slice(-20).join("\n");
      return {
        exitCode: result.exitCode,
        status: result.exitCode === 0 ? "clean" : "warnings/errors",
        summary: lastLines,
      };
    }

    // ── check_format ─────────────────────────────────────────────────
    case "check_format": {
      const result = runCommand("cargo", ["fmt", "--all", "--", "--check"]);
      return {
        exitCode: result.exitCode,
        status: result.exitCode === 0 ? "formatted" : "needs formatting",
        stdout: result.stdout.slice(0, 2000),
        stderr: result.stderr.slice(0, 2000),
      };
    }

    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}

// ── Main MCP loop ───────────────────────────────────────────────────────────

async function main() {
  // Process requests
  while (true) {
    const line = await readLine();
    if (!line) break;

    let req: JsonRpcRequest;
    try {
      req = JSON.parse(line);
    } catch {
      continue;
    }

    if (req.method === "initialize") {
      // Respond with the protocol version the client requested
      const clientVersion = (req.params?.protocolVersion as string) || "2024-11-05";
      send({
        jsonrpc: "2.0",
        id: req.id,
        result: {
          protocolVersion: clientVersion,
          capabilities: {
            tools: {
              run_ci_pipeline: {
                description: "Run the local CI pipeline (workflow-agent)",
                inputSchema: {
                  type: "object",
                  properties: {
                    workflow: {
                      type: "string",
                      description: "Workflow file path (default: .github/workflows/ci.yml)",
                    },
                  },
                },
              },
              check_todos: {
                description: "Verify TODOs in .task-state.json",
                inputSchema: {
                  type: "object",
                  properties: {
                    state_file: {
                      type: "string",
                      description: "Path to task state file (default: .task-state.json)",
                    },
                  },
                },
              },
              run_tests: {
                description: "Run cargo tests",
                inputSchema: {
                  type: "object",
                  properties: {
                    args: {
                      type: "string",
                      description: "Extra cargo test args (default: --all-features --workspace)",
                    },
                  },
                },
              },
              run_clippy: {
                description: "Run cargo clippy linter",
                inputSchema: {
                  type: "object",
                  properties: {
                    args: {
                      type: "string",
                      description: "Extra cargo clippy args (default: --all-targets --all-features)",
                    },
                  },
                },
              },
              check_format: {
                description: "Check Rust code formatting (cargo fmt --check)",
                inputSchema: {
                  type: "object",
                  properties: {},
                },
              },
            },
          },
          serverInfo: { name: "cicd-server", version: "1.0.0" },
        },
      });
    } else if (req.method === "notifications/initialized") {
      // Client confirmed initialization — nothing to do
      continue;
    } else if (req.method === "tools/list") {
      send({
        jsonrpc: "2.0",
        id: req.id,
        result: {
          tools: [
            {
              name: "run_ci_pipeline",
              description: "Run the local CI pipeline via workflow-agent.ts",
              inputSchema: {
                type: "object",
                properties: {
                  workflow: {
                    type: "string",
                    description: "Workflow file (default: .github/workflows/ci.yml)",
                  },
                },
              },
            },
            {
              name: "check_todos",
              description: "Verify all TODOs in .task-state.json are completed",
              inputSchema: {
                type: "object",
                properties: {
                  state_file: {
                    type: "string",
                    description: "Path to task state JSON (default: .task-state.json)",
                  },
                },
              },
            },
            {
              name: "run_tests",
              description: "Run cargo test suite",
              inputSchema: {
                type: "object",
                properties: {
                  args: {
                    type: "string",
                    description: "Test args (default: --all-features --workspace)",
                  },
                },
              },
            },
            {
              name: "run_clippy",
              description: "Run cargo clippy lint check",
              inputSchema: {
                type: "object",
                properties: {
                  args: {
                    type: "string",
                    description: "Clippy args (default: --all-targets --all-features)",
                  },
                },
              },
            },
            {
              name: "check_format",
              description: "Check Rust code formatting with cargo fmt",
              inputSchema: {
                type: "object",
                properties: {},
              },
            },
          ],
        },
      });
    } else if (req.method === "tools/call") {
      const toolName = req.params?.name as string;
      const toolArgs = (req.params?.arguments as Record<string, unknown>) || {};

      try {
        const result = await handleToolCall(toolName, toolArgs);
        send({
          jsonrpc: "2.0",
          id: req.id,
          result: { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] },
        });
      } catch (err: any) {
        send({
          jsonrpc: "2.0",
          id: req.id,
          error: { code: -32000, message: err.message },
        });
      }
    } else {
      send({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: `Method not found: ${req.method}` } });
    }
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
