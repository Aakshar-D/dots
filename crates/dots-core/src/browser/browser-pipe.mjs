// Dots' private connection to a dot's Microsoft Edge (browser spec section 14), written by Dots
// into <data_dir>/browser/ and run as:
//
//   node browser-pipe.mjs <msedge.exe> <edge switches...>
//
// It starts Edge with --remote-debugging-pipe, so that Edge opens no port, and relays Edge's
// pipe (its file descriptors 3 and 4) to this program's stdin and stdout, byte for byte:
// messages are JSON, each ended by a NUL byte. When stdin ends (Dots stopped), Edge is asked
// to close, and ended after 5 s. When Edge ends, so does this program.
import { spawn } from 'node:child_process';

const [exe, ...args] = process.argv.slice(2);
// Without --edge-skip-compat-layer-relaunch, msedge.exe hands over to another process and
// the pipe is lost.
const edge = spawn(exe, ['--remote-debugging-pipe', '--edge-skip-compat-layer-relaunch', ...args], {
  stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'],
});
const toEdge = edge.stdio[3];
const fromEdge = edge.stdio[4];
let midMessage = false;
let closing = false;

edge.on('error', (e) => {
  process.stderr.write(`browser-pipe: ${e.message}\n`);
  process.exit(1);
});
edge.on('exit', (code) => {
  process.exitCode = code ?? 0;
  process.stdin.destroy();
  toEdge.destroy();
  fromEdge.destroy();
});
toEdge.on('error', () => {});
fromEdge.on('error', () => {});

process.stdin.on('data', (chunk) => {
  if (closing || chunk.length === 0) return;
  midMessage = chunk[chunk.length - 1] !== 0;
  if (!toEdge.write(chunk)) {
    process.stdin.pause();
    toEdge.once('drain', () => process.stdin.resume());
  }
});
fromEdge.on('data', (chunk) => {
  if (!process.stdout.write(chunk)) {
    fromEdge.pause();
    process.stdout.once('drain', () => fromEdge.resume());
  }
});

function close() {
  if (closing) return;
  closing = true;
  // A message cut short is ended first, so that Edge reads the next one.
  if (midMessage) toEdge.write('\0');
  toEdge.write(JSON.stringify({ id: 2147483647, method: 'Browser.close' }) + '\0');
  setTimeout(() => edge.kill(), 5000).unref();
}
process.stdin.on('end', close);
process.stdin.on('error', close);
process.stdout.on('error', close);
