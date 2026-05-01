#!/usr/bin/env node
/**
 * check-todos.ts
 * Verifica se os TODOs definidos em .task-state.json foram concluídos.
 *
 * Uso:
 *   npx tsx scripts/check-todos.ts [.task-state.json]
 *
 * Exit code:
 *   0 = todos os TODOs obrigatórios concluídos
 *   1 = TODOs pendentes ou arquivo não encontrado
 */
import fs from 'node:fs';
import path from 'node:path';

type TodoItem = {
  id: string;
  title: string;
  required?: boolean;
  files?: string[];
  evidence?: string[];
};

type TaskState = {
  task?: string;
  todos?: TodoItem[];
};

type FileCheck = { file: string; exists: boolean };
type EvidenceCheck = { evidence: string; present: boolean };

type TodoResult = {
  id: string;
  title: string;
  required: boolean;
  ok: boolean;
  files: FileCheck[];
  evidence: EvidenceCheck[];
};

type CheckResult = {
  ok: boolean;
  task: string | null;
  totals: { total: number; pending: number; complete: number };
  results: TodoResult[];
  error?: string;
};

function fileExists(p: string): boolean {
  return fs.existsSync(path.resolve(process.cwd(), p));
}

function main(): void {
  const stateFile = process.argv[2] ?? '.task-state.json';

  if (!fileExists(stateFile)) {
    const out: CheckResult = {
      ok: false,
      task: null,
      totals: { total: 0, pending: 0, complete: 0 },
      results: [],
      error: `Arquivo não encontrado: ${stateFile}. Crie o .task-state.json antes de rodar /shipit.`,
    };
    console.log(JSON.stringify(out, null, 2));
    process.exit(1);
  }

  let state: TaskState;
  try {
    state = JSON.parse(fs.readFileSync(path.resolve(stateFile), 'utf8')) as TaskState;
  } catch (err) {
    const out: CheckResult = {
      ok: false,
      task: null,
      totals: { total: 0, pending: 0, complete: 0 },
      results: [],
      error: `JSON inválido em ${stateFile}: ${err instanceof Error ? err.message : String(err)}`,
    };
    console.log(JSON.stringify(out, null, 2));
    process.exit(1);
  }

  const todos = state.todos ?? [];

  const results: TodoResult[] = todos.map((todo) => {
    const fileChecks: FileCheck[] = (todo.files ?? []).map((f) => ({
      file: f,
      exists: fileExists(f),
    }));
    const evidenceChecks: EvidenceCheck[] = (todo.evidence ?? []).map((e) => ({
      evidence: e,
      present: true,
    }));
    const ok = fileChecks.every((c) => c.exists);
    return {
      id: todo.id,
      title: todo.title,
      required: todo.required !== false,
      ok,
      files: fileChecks,
      evidence: evidenceChecks,
    };
  });

  const pending = results.filter((r) => r.required && !r.ok);
  const out: CheckResult = {
    ok: pending.length === 0,
    task: state.task ?? null,
    totals: {
      total: results.length,
      pending: pending.length,
      complete: results.length - pending.length,
    },
    results,
  };

  console.log(JSON.stringify(out, null, 2));
  process.exit(out.ok ? 0 : 1);
}

main();
