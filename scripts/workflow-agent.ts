#!/usr/bin/env node
/**
 * workflow-agent.ts
 * Executa steps `run:` de jobs do GitHub Actions localmente via Docker.
 * Saída: JSON estruturado linha a linha para consumo pelo agente OpenCode.
 *
 * Uso:
 *   npx tsx scripts/workflow-agent.ts <workflow.yml> [job-id] [--dry-run]
 *
 * Exemplos:
 *   npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml
 *   npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml go-test
 *   npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml go-test --dry-run
 */
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import YAML from 'yaml';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------
type Step = {
  name?: string;
  run?: string;
  env?: Record<string, string>;
  'working-directory'?: string;
  uses?: string;
  'continue-on-error'?: boolean;
};

type Job = {
  name?: string;
  env?: Record<string, string>;
  steps?: Step[];
  container?: string | { image?: string };
  defaults?: { run?: { shell?: string; 'working-directory'?: string } };
  needs?: string | string[];
};

type Workflow = { jobs?: Record<string, Job> };

// ---------------------------------------------------------------------------
// Jobs que requerem secrets/serviços externos — pular localmente
// Adicione aqui jobs específicos do seu projeto se necessário.
// ---------------------------------------------------------------------------
const SKIP_JOBS = new Set([
  'secrets-scan',
  'semgrep',
  'sonarcloud',
  'codeql',
  'snyk',
  'dependabot',
]);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function emit(event: Record<string, unknown>): void {
  process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), ...event }) + os.EOL);
}

function mergeEnv(...parts: Array<Record<string, string> | undefined>): Record<string, string> {
  return Object.assign({}, ...parts.filter(Boolean)) as Record<string, string>;
}

function resolveContainer(job: Job): string {
  if (!job.container) return 'ubuntu:22.04';
  if (typeof job.container === 'string') return job.container;
  return job.container.image ?? 'ubuntu:22.04';
}

function readWorkflow(file: string): Workflow {
  const raw = fs.readFileSync(path.resolve(file), 'utf8');
  return YAML.parse(raw) as Workflow;
}

// ---------------------------------------------------------------------------
// Runner via Docker
// ---------------------------------------------------------------------------
async function runStepInDocker(opts: {
  image: string;
  projectRoot: string;
  stepWorkingDir: string;
  shell: string;
  command: string;
  env: Record<string, string>;
}): Promise<number> {
  const relDir = path.relative(opts.projectRoot, opts.stepWorkingDir) || '.';
  const wrappedCommand = `cd ${JSON.stringify(relDir)} && ${opts.command}`;

  const args = [
    'run', '--rm',
    '--network', 'none',
    '-v', `${opts.projectRoot}:/workspace`,
    '-w', '/workspace',
    ...Object.entries(opts.env).flatMap(([k, v]) => ['-e', `${k}=${v}`]),
    opts.image,
    opts.shell, '-lc', wrappedCommand,
  ];

  return new Promise<number>((resolve, reject) => {
    const child = spawn('docker', args, { stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.on('data', (buf) => {
      for (const line of String(buf).split(/\r?\n/)) {
        if (line.trim()) emit({ type: 'stdout', line });
      }
    });
    child.stderr.on('data', (buf) => {
      for (const line of String(buf).split(/\r?\n/)) {
        if (line.trim()) emit({ type: 'stderr', line });
      }
    });
    child.on('close', (code) => resolve(code ?? 1));
    child.on('error', reject);
  });
}

// ---------------------------------------------------------------------------
// Job runner
// ---------------------------------------------------------------------------
async function runJob(
  jobId: string,
  job: Job,
  projectRoot: string,
  dryRun: boolean,
): Promise<number> {
  if (SKIP_JOBS.has(jobId)) {
    emit({ type: 'job_skipped', job: jobId, reason: 'requires_external_secrets_or_services' });
    return 0;
  }

  const image = resolveContainer(job);
  const shell = job.defaults?.run?.shell ?? 'bash';
  const jobDefaultWd = path.resolve(projectRoot, job.defaults?.run?.['working-directory'] ?? '.');
  const jobEnv = mergeEnv(job.env);

  emit({ type: 'job_started', job: jobId, name: job.name ?? jobId, image, shell });

  const steps = (job.steps ?? []).filter((s) => s.run);

  if (dryRun) {
    for (const [i, step] of steps.entries()) {
      emit({
        type: 'step_dry_run',
        job: jobId,
        step: step.name ?? `step_${i + 1}`,
        command: step.run,
        workingDir: step['working-directory'] ?? job.defaults?.run?.['working-directory'] ?? '.',
      });
    }
    emit({ type: 'job_finished', job: jobId, status: 'dry_run' });
    return 0;
  }

  for (const [i, step] of steps.entries()) {
    const stepName = step.name ?? `step_${i + 1}`;
    const stepEnv = mergeEnv(jobEnv, step.env);
    const stepWd = step['working-directory']
      ? path.resolve(projectRoot, step['working-directory'])
      : jobDefaultWd;

    emit({ type: 'step_started', job: jobId, step: stepName });

    const code = await runStepInDocker({
      image,
      projectRoot,
      stepWorkingDir: stepWd,
      shell,
      command: step.run!,
      env: stepEnv,
    });

    emit({ type: 'step_finished', job: jobId, step: stepName, exitCode: code });

    if (code !== 0 && !step['continue-on-error']) {
      emit({ type: 'job_finished', job: jobId, status: 'failed', failedStep: stepName });
      return 1;
    }
  }

  emit({ type: 'job_finished', job: jobId, status: 'success' });
  return 0;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const dryRun = args.includes('--dry-run');
  const filtered = args.filter((a) => a !== '--dry-run');

  const workflowPath = filtered[0];
  const targetJob = filtered[1];

  if (!workflowPath) {
    emit({ type: 'error', message: 'Uso: npx tsx scripts/workflow-agent.ts <workflow.yml> [job-id] [--dry-run]' });
    process.exit(1);
  }

  const workflow = readWorkflow(workflowPath);
  const projectRoot = process.cwd();
  const allJobs = workflow.jobs ?? {};

  let selected: Array<[string, Job]>;
  if (targetJob) {
    if (!allJobs[targetJob]) {
      emit({ type: 'error', message: `Job não encontrado: ${targetJob}`, available: Object.keys(allJobs) });
      process.exit(1);
    }
    selected = [[targetJob, allJobs[targetJob]]];
  } else {
    selected = Object.entries(allJobs) as Array<[string, Job]>;
  }

  let failures = 0;
  for (const [jobId, job] of selected) {
    const code = await runJob(jobId, job, projectRoot, dryRun);
    if (code !== 0) failures += 1;
  }

  emit({
    type: 'workflow_finished',
    status: failures === 0 ? 'success' : 'failed',
    failures,
  });

  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err: unknown) => {
  emit({ type: 'error', message: err instanceof Error ? err.message : String(err) });
  process.exit(1);
});
