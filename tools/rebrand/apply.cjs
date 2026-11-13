// Dots became Sage, and a dot a sol. This rewrites the old words where a person reads or
// hears them: string literals, JSX text and regex literals of .ts/.tsx, string literals of
// .rs, and the prose files listed in PROSE. Identifiers, comments, SQL, code inside strings
// and the texts that are signed or name something stored are left alone.
//
// It is kept for branches that were written before the change: run it on them after a
// merge with master, read `pairs.json` (every rewrite, and every string kept on purpose)
// and `bare.json` (single words such as "dot", which are enum values and keys unless they
// sit where a label does), then run the tests.
//
// Usage: node tools/rebrand/apply.cjs <repo root> --dry|--write [path prefix ...]
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const root = path.resolve(process.argv[2]);
// The UI's own TypeScript reads the .ts and .tsx files (`bun install` first).
const ts = require(process.env.TS_LIB || path.join(root, 'ui/node_modules/typescript'));
// The two reports go to the temp folder unless REBRAND_OUT names another.
const outDir = process.env.REBRAND_OUT || require('os').tmpdir();
const write = process.argv[3] === '--write';
const prefixes = process.argv.slice(4);
const overrides = fs.existsSync(path.join(__dirname, 'overrides.json'))
  ? JSON.parse(fs.readFileSync(path.join(__dirname, 'overrides.json'), 'utf8'))
  : {};

const SKIP_FILES = [/^crates\/dots-core\/migrations\//, /^docs\/superpowers\//, /^docs\/verification\//];

// Whole strings that must stay as they are.
const KEEP = [
  /\b(SELECT|INSERT INTO|INSERT OR|UPDATE|DELETE FROM|CREATE TABLE|CREATE INDEX|CREATE UNIQUE|ALTER TABLE|DROP TABLE|LEFT JOIN|JOIN) [\s\S]*\b(FROM|INTO|SET|TABLE|ON|VALUES)\b/,
  /(=>|\bfunction\s*\w*\(|\bconst \w+ =|\bexport (function|const|default)|\bctx\.\w|\bimport .* from)/, // code inside a string: by hand
  /\b(FROM|INTO|JOIN|EXISTS|TABLE) dots\b/,
  /^dots (policy|roster|request|identity|phone view|url|auth header)/, // signing and key-store names
  /ends in a dot\b/,
  /\bdot=/, /^\[dot\]$/, // log fields the problem report scrubs
  /user\.name=/,
  /^dots: /, // the program's name before a startup line
  /^dots\.$/, // the log file's name
  /\bdots (policy|roster|request) /, // the signed text, quoted in a test
  /Dots 0\.4\.0 and older/,
  /Dots checkpoint:/,
  /Dots release signing key/,
];

const SEP = '(?:\\s|\\\\\\r?\\n|&nbsp;)+';
const W = /(?<![\w\-./:#@$\\{'"`])(Dots|dots|DOTS|Dot|dot|DOT)(?![\w\-/#@\\}]|\.\w|:\S)/g;

const PLURAL_NEXT = new Set(['that', 'read', 'see', 'never', 'in']);
const PLURAL_PREV = new Set(['all', 'these', 'those', 'two', 'three']);
const SOL_NEXT = new Set(['created', 'deleted', 'settings', 'runs', 'name', 'names']);
const PERSONA_NEXT = new Set(['is', 'has', 'starts', 'stop', 'asks', 'said', 'says', 'heard', 'answers', 'speaks', 'listens']);
const PERSONA_PREV = new Set(['to', "i'm", 'am', 'are', 'hi', 'okay', 'ok', 'hello', 'thanks']);

const wordAfter = (s, i) => (/^(?:\s|\\\r?\n)*([A-Za-z']+)/.exec(s.slice(i)) || [])[1]?.toLowerCase();
const wordBefore = (s, i) => (/([A-Za-z']+)(?:\s|\\\r?\n)*$/.exec(s.slice(0, i)) || [])[1]?.toLowerCase();

const BARE = /^(Dots|dots|DOTS|Dot|dot|DOT)$/;

const mask = (s) => s.replace(/\\([nrt])/g, (m, c) => '\u0001' + { n: '\u0002', r: '\u0003', t: '\u0004' }[c]);

function rewrite(s, label) {
  if (!W.test(mask(s))) { W.lastIndex = 0; return s; }
  W.lastIndex = 0;
  const flat = s.replace(/\s+/g, ' ').trim();
  if (Object.prototype.hasOwnProperty.call(overrides, flat)) {
    const o = overrides[flat];
    return o === null ? s : o;
  }
  // A bare word is an enum value, a key or a path unless it sits where a label does.
  if (label === 'jsxPart') {
    // Text beside an expression, such as `Dots {version}`: the app's name, or one sol.
    if (flat === 'Dots') return s.replace('Dots', 'Sage');
    if (flat === 'Dot') return s.replace('Dot', 'Sol');
  } else if (BARE.test(label === 'jsxText' ? flat : s)) {
    if (label && flat === 'Dot') return s.replace('Dot', 'Sol');
    if (label && flat === 'Dots') return s.replace('Dots', 'Sols');
    return s;
  }
  if (KEEP.some((r) => r.test(s))) return s;

  // "\n", "\r" and "\t" are breaks, not letters: hide them while the words are matched.
  let t = mask(s);
  const phrase = (re, to) => {
    t = t.replace(re, (m, a, sep) => {
      const up = m === m.toUpperCase();
      const low = a[0] === a[0].toLowerCase();
      const second = up ? to.toUpperCase() : low ? to.toLowerCase() : to;
      return a + sep + second;
    });
  };
  phrase(new RegExp('(?<![\\w\\-])(Open|open|OPEN)(' + SEP + ')(?:Dot|dot|DOT)(?![\\w\\-]|\\.\\w)', 'g'), 'Sage');
  phrase(new RegExp('(?<![\\w\\-])(Hey|hey|HEY)(' + SEP + ')(?:Dot|dot|DOT)(?![\\w\\-]|\\.\\w)', 'g'), 'Sage');
  t = t.replace(new RegExp('(?<![\\w\\-])Dots(' + SEP + ')ID(?!\\w)', 'g'), 'Sage$1ID');

  t = t.replace(W, (m, w, i) => {
    const next = wordAfter(t, i + m.length);
    const prev = wordBefore(t, i);
    const rest = t.slice(i + m.length);
    if (w === 'Dots') {
      const single = ['a', 'this', 'each', 'every', 'another', 'installed', 'new', 'own'].includes(prev);
      const plural = !single && (PLURAL_NEXT.has(next) || PLURAL_PREV.has(prev));
      if (plural) return 'Sols';
      if (/^(&apos;|'|’)(?![a-z])/.test(rest)) return 'Sage__POSS__';
      return 'Sage';
    }
    if (w === 'Dot') return SOL_NEXT.has(next) || /^\s*[{\d]/.test(rest) ? 'Sol' : 'Sage';
    if (w === 'dots') return 'sols';
    if (w === 'dot') return 'sol';
    if (w === 'DOTS') return 'SAGE';
    return 'SOL';
  });
  t = t.replace(/Sage__POSS__(&apos;|'|’)/g, 'Sage$1s');
  return t.replace(/\u0001([\u0002\u0003\u0004])/g, (m, c) => '\\' + { '\u0002': 'n', '\u0003': 'r', '\u0004': 't' }[c]);
}

const files = execSync('git ls-files', { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 })
  .split('\n').filter(Boolean)
  .filter((f) => /\.(ts|tsx|rs)$/.test(f))
  .filter((f) => !SKIP_FILES.some((r) => r.test(f)))
  .filter((f) => !prefixes.length || prefixes.some((p) => f.startsWith(p)));

const pairs = new Map();
const bare = [];
let changedFiles = 0, changedSpans = 0;

function note(file, line, oldT, newT) {
  const flat = oldT.replace(/\s+/g, ' ').trim();
  if (BARE.test(flat) && oldT === newT) { bare.push({ file, line, text: flat }); return; }
  if (oldT === newT) {
    W.lastIndex = 0;
    if (W.test(oldT)) { const k = 'KEPT ' + flat; pairs.set(k, pairs.get(k) || { old: flat, new: null, n: 0, file }); pairs.get(k).n++; }
    W.lastIndex = 0;
    return;
  }
  const e = pairs.get(flat) || { old: flat, new: newT.replace(/\s+/g, ' ').trim(), n: 0, file };
  e.n++; pairs.set(flat, e);
}

// Each collector returns spans [start, end) of raw literal content to rewrite.
function spansTs(file, src) {
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true,
    file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const spans = [];
  const S = ts.SyntaxKind;
  const LABEL_ATTR = /^(aria-label|label|title|placeholder|alt|heading|description|name)$/;
  // A bare word is a label when it is a JSX attribute people read, or what a test looks up on screen.
  const labelOf = (n) => {
    const p = n.parent;
    if (p && p.kind === S.JsxAttribute) return LABEL_ATTR.test(p.name.getText(sf)) && p.name.getText(sf) !== 'name' ? 'attr' : undefined;
    if (!/\.test\.tsx?$/.test(file)) return undefined;
    for (let a = p, d = 0; a && d < 4; a = a.parent, d++) {
      if (a.kind === S.CallExpression && /(get|find|query)(All)?By|toHaveTextContent|toHaveAccessibleName/.test(a.expression.getText(sf))) return 'lookup';
    }
    return undefined;
  };
  const visit = (n) => {
    const k = n.kind;
    if (k === S.StringLiteral || k === S.NoSubstitutionTemplateLiteral) spans.push([n.getStart(sf) + 1, n.getEnd() - 1, labelOf(n)]);
    else if (k === S.TemplateHead || k === S.TemplateMiddle) spans.push([n.getStart(sf) + 1, n.getEnd() - 2]);
    else if (k === S.TemplateTail) spans.push([n.getStart(sf) + 1, n.getEnd() - 1]);
    else if (k === S.JsxText) spans.push([n.getFullStart(), n.getEnd(), n.parent.children.length === 1 ? 'jsxText' : 'jsxPart']);
    else if (k === S.RegularExpressionLiteral) spans.push([n.getStart(sf) + 1, src.lastIndexOf('/', n.getEnd())]);
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return spans;
}

function spansRust(src) {
  const spans = [];
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') { while (i < n && src[i] !== '\n') i++; continue; }
    if (c === '/' && src[i + 1] === '*') {
      let depth = 1; i += 2;
      while (i < n && depth) {
        if (src[i] === '/' && src[i + 1] === '*') { depth++; i += 2; continue; }
        if (src[i] === '*' && src[i + 1] === '/') { depth--; i += 2; continue; }
        i++;
      }
      continue;
    }
    const raw = /^b?r(#*)"/.exec(src.slice(i, i + 12));
    if (raw && !/\w/.test(src[i - 1] || ' ')) {
      const close = '"' + raw[1];
      const start = i + raw[0].length;
      const end = src.indexOf(close, start);
      if (end < 0) break;
      spans.push([start, end]);
      i = end + close.length;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') { if (src[j] === '\\') j++; j++; }
      spans.push([i + 1, j]);
      i = j + 1;
      continue;
    }
    if (c === "'") {
      const m = /^'(\\.[^']*|[^'\\])'/.exec(src.slice(i, i + 12));
      i += m ? m[0].length : 1;
      continue;
    }
    i++;
  }
  return spans;
}

for (const file of files) {
  const full = path.join(root, file);
  const src = fs.readFileSync(full, 'utf8');
  const spans = file.endsWith('.rs') ? spansRust(src) : spansTs(file, src);
  spans.sort((a, b) => a[0] - b[0]);
  let outSrc = '', last = 0, touched = false;
  for (const [a, b, label] of spans) {
    if (a < last || b <= a) continue;
    const oldT = src.slice(a, b);
    const newT = rewrite(oldT, label);
    const line = src.slice(0, a).split('\n').length;
    note(file, line, oldT, newT);
    outSrc += src.slice(last, a) + newT;
    last = b;
    if (newT !== oldT) { touched = true; changedSpans++; }
  }
  outSrc += src.slice(last);
  if (touched) {
    changedFiles++;
    if (write) fs.writeFileSync(full, outSrc);
  }
}

// Prose files: every line is text, except fenced code and `inline code`.
const PROSE = /^(ui\/src\/api\/__fixtures__\/contract\.json|docs\/[^/]+\.md|release-notes\.md|crates\/dots-core\/apps\/projects\/(README\.md|app\.json)|crates\/dots-core\/src\/phone\/assets\/index\.html|ui\/index\.html|apps\/dots-app\/Info\.plist)$/;
const proseFiles = execSync('git ls-files', { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28 })
  .split('\n').filter((f) => PROSE.test(f))
  .filter((f) => !prefixes.length || prefixes.some((p) => f.startsWith(p)));
for (const file of proseFiles) {
  const full = path.join(root, file);
  const src = fs.readFileSync(full, 'utf8');
  let fenced = false;
  const outSrc = src.split('\n').map((ln, idx) => {
    if (/^\s*```/.test(ln)) { fenced = !fenced; return ln; }
    if (fenced) return ln;
    return ln.split(/(`[^`]*`)/).map((part) => {
      if (part.startsWith('`')) return part;
      const next = rewrite(part, 'jsxText');
      note(file, idx + 1, part, next);
      if (next !== part) changedSpans++;
      return next;
    }).join('');
  }).join('\n');
  if (outSrc !== src) { changedFiles++; if (write) fs.writeFileSync(full, outSrc); }
}

fs.writeFileSync(path.join(outDir, 'pairs.json'), JSON.stringify([...pairs.values()], null, 1));
fs.writeFileSync(path.join(outDir, 'bare.json'), JSON.stringify(bare, null, 1));
console.log('reports: pairs.json and bare.json in', outDir);
console.log(write ? 'WROTE' : 'DRY', changedSpans, 'spans in', changedFiles, 'files;', pairs.size, 'unique;', bare.length, 'bare tokens left for hand review');
