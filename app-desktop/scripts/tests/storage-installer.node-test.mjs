import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const compiler = process.env.NSIS_MAKENSIS || path.join(process.env.LOCALAPPDATA || '', 'tauri/NSIS/makensis.exe');

test('NSIS storage hooks preserve Unicode paths, upgrade choices, and user data', {
  skip: process.platform !== 'win32' || !existsSync(compiler),
  timeout: 120_000,
}, () => {
  const cache = path.join(root, '.release-cache');
  mkdirSync(cache, { recursive: true });
  const fixture = mkdtempSync(path.join(cache, 'storage-installer-'));
  const registryKey = path.win32.join('Software', 'JC-AI-Storage-Tests', path.basename(fixture));
  const setup = path.join(fixture, 'setup.exe');
  const install = path.join(fixture, 'app');
  const data = path.join(fixture, 'Dữ liệu có dấu');
  const nsisQuote = value => value.replaceAll('$', '$$').replaceAll('"', '$\\"');
  const script = `Unicode true
RequestExecutionLevel user
!include "MUI2.nsh"
!define JC_STORAGE_KEY "${registryKey}"
!include "${nsisQuote(path.join(root, 'src-tauri/windows/storage-hooks.nsh'))}"
Var PassiveMode
Name "JC AI storage test"
OutFile "${nsisQuote(setup)}"
InstallDir "${nsisQuote(install)}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Vietnamese"
Function .onInit
  SetShellVarContext current
FunctionEnd
Section
  !insertmacro NSIS_HOOK_PREINSTALL
  CreateDirectory "$INSTDIR"
  !insertmacro NSIS_HOOK_POSTINSTALL
  WriteUninstaller "$INSTDIR\\uninstall.exe"
SectionEnd
Section Uninstall
  Delete "$INSTDIR\\uninstall.exe"
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
`;
  try {
    const source = path.join(fixture, 'test.nsi');
    writeFileSync(source, script, 'utf8');
    const compiled = execFileSync(compiler, ['/V3', '/INPUTCHARSET', 'UTF8', source], { encoding: 'utf8', windowsHide: true });
    assert.doesNotMatch(compiled, /warning 6000:/, 'no unknown variable references before Tauri declares its variables');
    const run = args => {
      const result = spawnSync(setup, ['/S', ...args], { encoding: 'utf8', windowsHide: true, timeout: 20_000 });
      assert.ifError(result.error);
      return result.status;
    };
    const selected = () => readFileSync(path.join(install, 'jc-ai-data-dir.txt')).toString('utf16le').replace(/^\uFEFF/, '').trimEnd();
    assert.equal(run([`/DATADIR=${data}`]), 0);
    assert.equal(selected(), data);
    writeFileSync(path.join(data, 'project.json'), 'preserve me');
    assert.equal(run([]), 0, 'reinstall remembers selected data folder');
    assert.equal(selected(), data);
    assert.equal(readFileSync(path.join(data, 'project.json'), 'utf8'), 'preserve me');
    assert.equal(run(['/DATADIR=relative-data']), 2, 'relative path is rejected');
    assert.equal(run([`/DATADIR=${path.join(install, 'data')}`]), 2, 'data inside installation directory is rejected');
    assert.equal(selected(), data, 'invalid selection leaves prior configuration intact');
    const uninstall = spawnSync(path.join(install, 'uninstall.exe'), ['/S', `_?=${install}`], { windowsHide: true, timeout: 20_000 });
    assert.ifError(uninstall.error);
    assert.equal(uninstall.status, 0);
    assert.equal(existsSync(path.join(install, 'jc-ai-data-dir.txt')), false);
    assert.equal(readFileSync(path.join(data, 'project.json'), 'utf8'), 'preserve me');
  } finally {
    spawnSync('reg.exe', ['delete', path.win32.join('HKCU', registryKey), '/f'], { windowsHide: true });
    // Only delete this newly-created fixture, within the workspace cache.
    assert.ok(fixture.startsWith(`${cache}${path.sep}`));
    rmSync(fixture, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});
