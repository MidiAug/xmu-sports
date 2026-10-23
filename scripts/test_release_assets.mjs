// Exercise the actual workflow preparation block with a fake installer. No build/install.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";
import { randomUUID, createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
const workflow=readFileSync('.github/workflows/package.yml','utf8');
const version=JSON.parse(readFileSync('package.json','utf8')).version;
const block=workflow.match(/- name: Generate checksums and provenance\r?\n\s+shell: pwsh\r?\n\s+run: \|\r?\n([\s\S]*?)(?=      - uses:)/)?.[1];
assert.ok(block,'Missing workflow preparation block');
const root=resolve('artifacts/release-asset-test',randomUUID());mkdirSync(root,{recursive:true});
const original=`XMU Sports_${version}_x64-setup.exe`;
const canonical=`XMU.Sports_${version}_x64-setup.exe`;
const payload=Buffer.from('fake installer; never execute');
writeFileSync(join(root,original),payload);
const psLiteral=value=>"'"+value.replaceAll('\\','/').replaceAll("'","''")+"'";
const source="$ErrorActionPreference='Stop'\n"+block.replace(/^          /gm,'')
  .replaceAll('src-tauri/target/release/bundle/nsis/SHA256SUMS.txt',psLiteral(join(root,'SHA256SUMS.txt')))
  .replaceAll('src-tauri/target/release/bundle/nsis',psLiteral(root))
  .replaceAll('artifacts/release-notes.txt',psLiteral(join(root,'release-notes.txt')));
const file=join(root,'prepare.ps1');writeFileSync(file,source);
execFileSync('pwsh',['-NoProfile','-NonInteractive','-File',file],{windowsHide:true,stdio:'pipe',env:{...process.env,GITHUB_SHA:'release-test-source'}});
assert.equal(existsSync(join(root,original)),false);
assert.deepEqual(readFileSync(join(root,canonical)),payload);
const hash=createHash('sha256').update(payload).digest('hex');
assert.equal(readFileSync(join(root,'SHA256SUMS.txt'),'utf8').replace(/^\uFEFF/,'').trim(),`${hash}  ${canonical}`);
assert.match(readFileSync(join(root,'release-notes.txt'),'utf8'),/Source commit: release-test-source/);
console.log('Release asset contract passed: workflow normalizes GitHub filename before SHA-256, preserves bytes and writes release notes');
