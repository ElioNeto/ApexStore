import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { shellCommand } from './shell-command.js';

test('run steps propagate command and pipeline failures without continuing', () => {
  for (const command of ['false; echo unexpected', 'false | cat; echo unexpected']) {
    const [shell, ...args] = shellCommand('bash', command);
    const result = spawnSync(shell, args, { encoding: 'utf8' });
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.stdout, '');
  }
});

test('a job with only unsupported actions cannot report success', () => {
  const directory = mkdtempSync(join(tmpdir(), 'workflow-agent-'));
  try {
    const workflow = join(directory, 'ci.yml');
    writeFileSync(workflow, 'jobs:\n  lint:\n    steps:\n      - uses: rhysd/actionlint@v1.7.12\n');
    const result = spawnSync(process.execPath,
      ['scripts/node_modules/tsx/dist/cli.mjs', 'scripts/workflow-agent.ts', workflow],
      { encoding: 'utf8' });
    assert.equal(result.status, 1, result.stderr);
    const events = result.stdout.trim().split('\n').map(line => JSON.parse(line));
    assert.equal(events.find(event => event.type === 'job_finished').status, 'failed');
    assert.equal(events.find(event => event.type === 'workflow_finished').status, 'failed');
    assert(!events.some(event => event.status === 'success'));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
