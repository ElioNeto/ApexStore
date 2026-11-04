/** Match GitHub Actions' failure propagation for bash run steps. */
export function shellCommand(shell: string, command: string): string[] {
  return shell === 'bash'
    ? [shell, '-e', '-o', 'pipefail', '-c', command]
    : [shell, '-e', '-c', command];
}
