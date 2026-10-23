import assert from "node:assert/strict";
import {readFileSync,writeFileSync,mkdirSync,existsSync} from "node:fs";
import {execFileSync} from "node:child_process";
import {resolve,join} from "node:path";
import {randomUUID} from "node:crypto";

// Compile and run only the installer's upgrade-decision functions. This never
// installs the application or touches its registry keys, binaries or user data.
const root=resolve("artifacts/installer-upgrade-test");mkdirSync(root,{recursive:true});
const template=readFileSync("src-tauri/windows/installer.nsi","utf8");
const config=JSON.parse(readFileSync("src-tauri/tauri.conf.json","utf8"));
const installSection=template.indexOf('Section Install');
assert.ok(installSection>=0);
assert.ok(template.indexOf('!insertmacro CheckIfAppIsRunning',installSection) < template.indexOf('!insertmacro NSIS_HOOK_PREINSTALL',installSection),"stop the host before removing its dependencies");
assert.equal(config.productName,"XMU Sports");
assert.equal(config.identifier,"local.xmu.sports");
assert.equal(config.mainBinaryName,"xmu-sports");
const defines={PRODUCTNAME:config.productName,MANUFACTURER:config.bundle.publisher};
for(const key of ["INTERNALNAME","UNINSTKEY","MANUKEY","MANUPRODUCTKEY"]){
 const raw=template.match(new RegExp(`^!define ${key} "([^"\\r\\n]*)"`,"m"))?.[1];assert.ok(raw,key);
 defines[key]=raw.replace(/\$\{(\w+)\}/g,(_,name)=>defines[name]);
}
assert.equal(defines.UNINSTKEY,"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\xmu-sports");
assert.equal(defines.MANUPRODUCTKEY,"Software\\xmu\\xmu-sports");
const extract=name=>{const match=template.match(new RegExp(`^Function ${name}\\r?\\n[\\s\\S]*?^FunctionEnd`,"m"));assert.ok(match,`missing ${name}`);return match[0];};
const nsis=join(process.env.LOCALAPPDATA,"tauri/NSIS");
assert.ok(existsSync(join(nsis,"makensis.exe")),"Tauri's bundled NSIS is required");
const registry=`Software\\XmuSportsInstallerTests\\${randomUUID()}`;
const result=join(root,"result.ini");
const lines=[
  "Unicode true",'!include "LogicLib.nsh"','!include "Win\\COM.nsh"','!include "Win\\Propkey.nsh"',
  `!include "${resolve("src-tauri/target/release/nsis/x64/utils.nsh")}"`,
  `!include "${resolve("src-tauri/windows/hooks.nsh")}"`,`!addplugindir "${join(nsis,"Plugins/x86-unicode/additional")}"`,
  'Name "XMU installer decision test"',`OutFile "${join(root,"upgrade-decision-test.exe")}"`,
  "RequestExecutionLevel user","SilentInstall silent",'!define PRODUCTNAME "XMU Sports"','!define INTERNALNAME "xmu-sports"','!define MAINBINARYNAME "xmu-sports"','!define BUNDLEID "local.xmu.sports"',`!define VERSION "${config.version}"`,
  `!define UNINSTKEY "${registry}\\uninstall"`,`!define MANUPRODUCTKEY "${registry}\\product"`,
  "Var InPlaceUpdate","Var UpdateMode",extract("DetectInPlaceUpdate"),extract("RestorePreviousInstallLocation"),
  "Section", "SetShellVarContext current", "SetRegView 64", "StrCpy $InPlaceUpdate 0","StrCpy $UpdateMode 0",
  "Call DetectInPlaceUpdate",`WriteINIStr "${result}" "fresh" "inplace" $InPlaceUpdate`,
  'WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" "test-only-uninstaller.exe"',
];
for (const [name,version] of [["upgrade","0.1.8"],["repair",config.version],["downgrade","0.2.0"]]) {
  lines.push("StrCpy $InPlaceUpdate 0","StrCpy $UpdateMode 0",`WriteRegStr HKCU "\${UNINSTKEY}" "DisplayVersion" "${version}"`,"Call DetectInPlaceUpdate",
    `WriteINIStr "${result}" "${name}" "inplace" $InPlaceUpdate`,`WriteINIStr "${result}" "${name}" "update" $UpdateMode`);
}
lines.push(`WriteRegStr HKCU "\${MANUPRODUCTKEY}" "" "${join(root,"existing-install")}"`,"Call RestorePreviousInstallLocation",
  `WriteINIStr "${result}" "location" "existing" $INSTDIR`,
  `WriteRegStr HKCU "\${MANUPRODUCTKEY}" "" "${join(process.env.LOCALAPPDATA,"xmu-sports")}"`,"Call RestorePreviousInstallLocation",
  `WriteINIStr "${result}" "location" "legacy" $INSTDIR`,`DeleteRegKey HKCU "${registry}"`,
  `StrCpy $INSTDIR "${join(root,"shortcut-app")}"`,'CreateDirectory "$INSTDIR"',
  'FileOpen $0 "$INSTDIR\\xmu-sports.exe" w','FileClose $0',
  `CreateDirectory "${join(root,"shortcuts")}"`,
  `CreateShortcut "${join(root,"shortcuts/xmu-sports.lnk")}" "$INSTDIR\\xmu-sports.exe"`,
  `!insertmacro XMU_MIGRATE_SHORTCUT "${join(root,"shortcuts")}"`,
  `CreateDirectory "${join(root,"previous-brand-shortcuts")}"`,
  `CreateShortcut "${join(root,"previous-brand-shortcuts/XMU Sport.lnk")}" "$INSTDIR\\xmu-sports.exe"`,
  `!insertmacro XMU_MIGRATE_SHORTCUT "${join(root,"previous-brand-shortcuts")}"`,
  `StrCpy $INSTDIR "${join(root,"lite-upgrade")}"`);
const owned=["xmu-sports.exe","http-booking/xmu-http-booking.exe","http-booking/_internal/frida.dll","ocr/README.md","ocr/old-model.onnx","vision-pack/manifest.json","tasks/task.json","runs/run.json","http-booking/requests/journal.json"];
for(const file of owned){const path=join(root,"lite-upgrade",file);mkdirSync(resolve(path,".."),{recursive:true});writeFileSync(path,"test sentinel");}
lines.push('!insertmacro XMU_REMOVE_OBSOLETE_COMPONENTS',`StrCpy $INSTDIR "${join(root,"unowned-resources")}"`);
for(const file of ["xmu-sports.exe","ocr/personal.txt","http-booking/_internal/personal.txt"]){const path=join(root,"unowned-resources",file);mkdirSync(resolve(path,".."),{recursive:true});writeFileSync(path,"keep");}
lines.push('!insertmacro XMU_REMOVE_OBSOLETE_COMPONENTS',`StrCpy $INSTDIR "${join(root,"partial-install")}"`);
for(const file of ["xmu-sports.exe","http-booking/xmu-http-booking.exe"]){const path=join(root,"partial-install",file);mkdirSync(resolve(path,".."),{recursive:true});writeFileSync(path,"partial install");}
lines.push('!insertmacro XMU_REMOVE_OBSOLETE_COMPONENTS',`WriteINIStr "${result}" "cleanup" "completed" "1"`,"SectionEnd");
const source=join(root,"upgrade-decision-test.nsi");writeFileSync(source,lines.join("\n"));
execFileSync(join(nsis,"makensis.exe"),["/V2",source],{stdio:"pipe",windowsHide:true});
execFileSync(join(root,"upgrade-decision-test.exe"),[],{stdio:"pipe",windowsHide:true});
const bytes=readFileSync(result);
const ini=bytes.toString(bytes[0]===0xff&&bytes[1]===0xfe?"utf16le":"utf8");
const value=(section,key)=>ini.match(new RegExp(`\\[${section}\\]([\\s\\S]*?)(?=\\[|$)`))?.[1].match(new RegExp(`^${key}=(.*)$`,"m"))?.[1].trim();
assert.equal(value("fresh","inplace"),"0");
assert.equal(value("cleanup","completed"),"1","repair tolerates an already missing obsolete directory");
for(const kind of ["upgrade","repair"]){assert.equal(value(kind,"inplace"),"1");assert.equal(value(kind,"update"),"1");}
assert.equal(value("downgrade","inplace"),"0");
assert.equal(value("location","existing"),join(root,"existing-install"));
assert.equal(value("location","legacy"),join(process.env.LOCALAPPDATA,"xmu-sports"));
assert.equal(existsSync(join(root,"shortcuts/XMU Sports.lnk")),true,"upgrade renames the existing owned shortcut");
assert.equal(existsSync(join(root,"shortcuts/xmu-sports.lnk")),false,"the old app shortcut must not remain duplicated");
assert.equal(existsSync(join(root,"previous-brand-shortcuts/XMU Sports.lnk")),true,"upgrade renames the previous display brand shortcut");
assert.equal(existsSync(join(root,"previous-brand-shortcuts/XMU Sport.lnk")),false,"the previous display brand shortcut must not remain duplicated");
for(const file of ["http-booking/_internal","ocr","vision-pack"]){assert.equal(existsSync(join(root,"lite-upgrade",file)),false,`obsolete component removed: ${file}`);}
for(const file of ["tasks/task.json","runs/run.json","http-booking/requests/journal.json","http-booking/xmu-http-booking.exe"]){assert.equal(readFileSync(join(root,"lite-upgrade",file),"utf8"),"test sentinel",`user data preserved: ${file}`);}
for(const file of ["ocr/personal.txt","http-booking/_internal/personal.txt"]){assert.equal(readFileSync(join(root,"unowned-resources",file),"utf8"),"keep","unowned directories preserved");}
console.log("NSIS upgrade checks passed: upgrade/repair/downgrade, paths, shortcuts, obsolete component removal and preservation of tasks, runs, request journals and unowned folders");
