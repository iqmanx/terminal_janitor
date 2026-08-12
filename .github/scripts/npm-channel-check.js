#!/usr/bin/env node
// Guards the npm channel on every push and pull request.
//
// The release workflow is the only thing that ever exercises these packages,
// and it runs only on a tag. A mistake found there is already a published
// GitHub release whose npm half is missing or broken, and npm will not let the
// same version be published twice. So the parts that can be checked without a
// tag are checked here instead: they run offline in about a second, need no
// Rust build, and cover the two properties nothing else does — the supported
// platform list is identical everywhere it is written down, and the wrapper
// really does hand control to the binary it resolves, exit code and all.
//
// Run it directly at any time: `node .github/scripts/npm-channel-check.js`.

'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const root = path.resolve(__dirname, '..', '..');
const wrapperDir = path.join(root, 'npm', 'terminal_janitor');
const wrapperBin = path.join(wrapperDir, 'bin', 'terminal_janitor.js');
const workflow = path.join(root, '.github', 'workflows', 'release.yml');

const failures = [];
const fail = (message) => failures.push(message);
const ok = (message) => process.stdout.write(`  ok  ${message}\n`);
const skip = (message) => process.stdout.write(`  --  ${message}\n`);

const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
const sorted = (values) => [...new Set(values)].sort();
const same = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);

// --- the platform list, in all four places it is written ------------------

// The wrapper's table is the declaration a user's machine is matched against.
// It is read out of the source rather than imported, because importing this
// file runs it.
const wrapperSource = fs.readFileSync(wrapperBin, 'utf8');
const table = sorted(
  [...wrapperSource.matchAll(/'(terminal_janitor-[a-z0-9-]+)'/g)].map(
    (match) => match[1],
  ),
);

const directories = sorted(
  fs
    .readdirSync(path.join(root, 'npm'))
    .filter((entry) => entry.startsWith('terminal_janitor-')),
);

const wrapperManifest = readJson(path.join(wrapperDir, 'package.json'));
const optional = sorted(Object.keys(wrapperManifest.optionalDependencies ?? {}));

// The workflow has to unpack an artefact into each package and publish each
// one; a package it never names is built and then silently dropped.
const workflowSource = fs.readFileSync(workflow, 'utf8');
const inWorkflow = sorted(
  [...workflowSource.matchAll(/terminal_janitor-[a-z0-9-]+/g)]
    .map((match) => match[0])
    // The archive names carry Rust target triples, not package names.
    .filter((name) => directories.includes(name)),
);

if (table.length === 0) {
  fail('no platform packages found in the wrapper table');
}

for (const [label, list] of [
  ['the platform package directories', directories],
  ["the wrapper's optionalDependencies", optional],
  ['the release workflow', inWorkflow],
]) {
  if (!same(table, list)) {
    fail(
      `${label} do not match the wrapper's platform table.\n` +
        `      table: ${table.join(', ')}\n` +
        `      found: ${list.join(', ')}`,
    );
  }
}
if (failures.length === 0) {
  ok(`the same ${table.length} platforms are named in all four places`);
}

// --- one version across the set -------------------------------------------

// The release workflow stamps the tag over every version and pins the
// wrapper's optional dependencies to it exactly. These are the committed
// placeholders, and they still have to agree: a stale one here is a package
// published against a version it was never built with.
const version = wrapperManifest.version;
for (const name of directories) {
  let manifest;
  try {
    manifest = readJson(path.join(root, 'npm', name, 'package.json'));
  } catch (error) {
    fail(`npm/${name} has no readable package.json: ${error.message}`);
    continue;
  }

  if (manifest.name !== name) {
    fail(`npm/${name} declares the name ${manifest.name}`);
  }
  if (manifest.version !== version) {
    fail(
      `${name} is version ${manifest.version}, but the wrapper is ${version}`,
    );
  }
  if (optional.includes(name) && wrapperManifest.optionalDependencies[name] !== version) {
    fail(
      `the wrapper pins ${name} at ` +
        `${wrapperManifest.optionalDependencies[name]}, not ${version}`,
    );
  }

  // `os` and `cpu` are what makes npm install exactly one of these packages.
  // Without them every machine installs all five, and the wrapper picks a
  // binary that cannot run.
  const [, platform, arch] = name.split('-');
  if (!Array.isArray(manifest.os) || !manifest.os.includes(platform)) {
    fail(`${name} does not declare "os": ["${platform}"]`);
  }
  if (!Array.isArray(manifest.cpu) || !manifest.cpu.includes(arch)) {
    fail(`${name} does not declare "cpu": ["${arch}"]`);
  }
  if (!(manifest.files ?? []).includes('bin/')) {
    fail(`${name} does not ship bin/ in its "files"`);
  }
  if (manifest.license !== wrapperManifest.license) {
    fail(`${name} declares a different licence from the wrapper`);
  }
}
ok(`every package is version ${version} and selects itself by os and cpu`);

// --- the workflow publishes directories, not repositories -------------------

// npm resolves a bare `a/b` argument as the GitHub shorthand for a repository
// before it considers a directory of that name, so `npm publish npm/<package>`
// reaches for github.com/npm/<package> and fails on an SSH key. Only a path
// beginning with `./`, `../`, `/` or `~/` is read as a directory. The mistake
// costs nothing to make, survives every other check in this file, and cannot
// surface before a tag, because publishing is the one step no earlier job runs.
const publishArguments = [
  ...workflowSource.matchAll(/npm publish\s+"?([^"\s]+)"?/g),
].map((match) => match[1]);

if (publishArguments.length === 0) {
  fail('the release workflow contains no npm publish command');
} else {
  const bare = publishArguments.filter((arg) => !/^([.]{1,2}|~)?\//.test(arg));
  if (bare.length > 0) {
    fail(
      `the release workflow publishes ${bare.join(', ')}, which npm reads as ` +
        'a GitHub repository rather than a directory; prefix the path with ./',
    );
  } else {
    ok(`all ${publishArguments.length} npm publish paths are directories`);
  }
}

// --- the wrapper's own manifest -------------------------------------------

if (wrapperManifest.bin?.terminal_janitor !== 'bin/terminal_janitor.js') {
  fail('the wrapper does not expose bin/terminal_janitor.js as its binary');
}
if (!(wrapperManifest.files ?? []).includes('bin/terminal_janitor.js')) {
  fail('the wrapper does not ship bin/terminal_janitor.js in its "files"');
}
if (wrapperManifest.os || wrapperManifest.cpu) {
  fail('the wrapper restricts os or cpu, so some machines cannot install it');
}

const syntax = spawnSync(process.execPath, ['--check', wrapperBin], {
  encoding: 'utf8',
});
if (syntax.status !== 0) {
  fail(`the wrapper does not parse:\n${syntax.stderr.trim()}`);
} else {
  ok('the wrapper parses and declares itself correctly');
}

// --- what the wrapper actually does ---------------------------------------

// Everything above reads files. This part installs the real layout npm would
// produce — the wrapper in node_modules beside a platform package — puts a
// stub where the native binary goes, and runs it.
const platformPackage = table.find(
  (name) => name === `terminal_janitor-${process.platform}-${process.arch}`,
);

if (process.platform === 'win32') {
  skip('behaviour checks need a shell stub; not run on Windows');
} else if (!platformPackage) {
  skip(`no platform package for ${process.platform} ${process.arch}`);
} else {
  const layout = (withBinary) => {
    const base = fs.mkdtempSync(path.join(os.tmpdir(), 'tj-npm-'));
    const installed = path.join(base, 'node_modules', 'terminal_janitor', 'bin');
    fs.mkdirSync(installed, { recursive: true });
    fs.copyFileSync(wrapperBin, path.join(installed, 'terminal_janitor.js'));
    fs.copyFileSync(
      path.join(wrapperDir, 'package.json'),
      path.join(installed, '..', 'package.json'),
    );

    if (withBinary) {
      const dir = path.join(base, 'node_modules', platformPackage);
      fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
      fs.copyFileSync(
        path.join(root, 'npm', platformPackage, 'package.json'),
        path.join(dir, 'package.json'),
      );
      const stub = path.join(dir, 'bin', 'terminal_janitor');
      fs.writeFileSync(stub, withBinary);
      fs.chmodSync(stub, 0o755);
    }
    return path.join(installed, 'terminal_janitor.js');
  };

  const run = (entry, args) =>
    spawnSync(process.execPath, [entry, ...args], { encoding: 'utf8' });

  // Arguments and exit codes pass through untouched. The exit code is the
  // contract the scheduler reads, so a wrapper that flattened 3 to 1 would
  // turn a reported shortfall into a reported failure.
  const passthrough = run(
    layout('#!/bin/sh\nprintf "stub:%s\\n" "$*"\nexit 3\n'),
    ['status', '--json'],
  );
  if (passthrough.stdout.trim() !== 'stub:status --json') {
    fail(`arguments did not reach the binary: ${JSON.stringify(passthrough.stdout)}`);
  }
  if (passthrough.status !== 3) {
    fail(`exit code 3 came back as ${passthrough.status}`);
  }
  ok('arguments, stdout, and the exit code pass through to the binary');

  // A binary killed by a signal Node ignores by default must not be reported
  // as a clean exit. SIGPIPE is that case, and 128 + 13 is the shell's answer.
  const signalled = run(layout('#!/bin/sh\nkill -PIPE $$\n'), []);
  if (signalled.status !== 141) {
    fail(
      'a binary killed by SIGPIPE should exit 141, not ' +
        `${signalled.status} (signal ${signalled.signal})`,
    );
  } else {
    ok('a binary killed by SIGPIPE exits 141 rather than reporting success');
  }

  // The optional dependency is exactly what --omit=optional and an unsupported
  // C library leave out, so this path is reachable in the wild and has to say
  // something a user can act on.
  const missing = run(layout(null), ['status']);
  if (missing.status !== 1 || !missing.stderr.includes('is not installed')) {
    fail(
      'a missing platform package should exit 1 with an explanation, got ' +
        `${missing.status}: ${missing.stderr.trim()}`,
    );
  } else {
    ok('a missing platform package fails with an explanation');
  }
}

// --- report ---------------------------------------------------------------

if (failures.length > 0) {
  process.stderr.write('\nnpm channel check failed:\n');
  for (const failure of failures) {
    process.stderr.write(`  - ${failure}\n`);
  }
  process.exit(1);
}
process.stdout.write('npm channel: all checks passed\n');
