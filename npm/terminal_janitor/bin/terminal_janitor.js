#!/usr/bin/env node
// Hands control to the native binary shipped in the matching platform package.
//
// This wrapper adds no behaviour of its own. Arguments, stdio, exit codes, and
// signals pass straight through, because a `terminal_janitor` exit code is a
// contract: the scheduler distinguishes a healthy machine from a shortfall from
// a failure by the number alone, and a wrapper that flattened those would make
// automatic operation lie.
//
// Nothing is downloaded here. npm selects the one platform package whose `os`
// and `cpu` match the machine, so an install works offline from a registry
// mirror and under `--ignore-scripts`.

'use strict';

const { spawnSync } = require('node:child_process');
const path = require('node:path');

// Every key is a `${process.platform} ${process.arch}` pair, and every value is
// a target the release workflow actually builds. A platform absent from this
// table has no artefact, so it is an explicit error rather than a guess at a
// binary that would not run.
const PLATFORM_PACKAGES = {
  'darwin arm64': 'terminal_janitor-darwin-arm64',
  'darwin x64': 'terminal_janitor-darwin-x64',
  'linux arm64': 'terminal_janitor-linux-arm64',
  'linux x64': 'terminal_janitor-linux-x64',
  'win32 x64': 'terminal_janitor-win32-x64',
};

function resolveBinary() {
  const platform = `${process.platform} ${process.arch}`;
  const packageName = PLATFORM_PACKAGES[platform];
  if (!packageName) {
    const supported = Object.keys(PLATFORM_PACKAGES).sort().join(', ');
    throw new Error(
      `unsupported platform ${platform}; this release supports ${supported}`,
    );
  }

  const executable =
    process.platform === 'win32' ? 'terminal_janitor.exe' : 'terminal_janitor';

  // Resolved through the package's own manifest rather than the executable
  // path, so this keeps working whatever the platform package declares in
  // `exports`.
  let manifest;
  try {
    manifest = require.resolve(`${packageName}/package.json`);
  } catch {
    throw new Error(
      `the ${packageName} package is missing. It is an optional dependency, ` +
        'so installing with --no-optional or --omit=optional skips it; ' +
        'reinstall without that flag.',
    );
  }
  return path.join(path.dirname(manifest), 'bin', executable);
}

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  process.stderr.write(`terminal_janitor: ${error.message}\n`);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  process.stderr.write(`terminal_janitor: ${result.error.message}\n`);
  process.exit(1);
}

// A binary killed by a signal did not choose an exit code, and reporting one
// would invent an outcome. Re-raise the same signal instead.
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status === null ? 1 : result.status);
}
