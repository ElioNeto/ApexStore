import { execSync } from 'child_process';
import { readFileSync } from 'fs';

// Read responses for git add -p
const responses = readFileSync('/tmp/teamcode/git-add-responses.txt', 'utf-8');
execSync('git add -p', { input: responses, cwd: '/mnt/data/projetos/ApexStore' });
console.log('Files staged successfully');
