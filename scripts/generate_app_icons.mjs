// Reuse the About/header mark on the application's white rounded tile.
import {readFileSync,writeFileSync,mkdirSync,readdirSync,copyFileSync,existsSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {execFileSync} from 'node:child_process';
const root=resolve(import.meta.dirname,'..');
const source=join(root,'assets/app-icon/source.svg');
const mark=readFileSync(join(root,'src/assets/brand-mark.png')).toString('base64');
const output=join(root,'artifacts/app-icon-generated');
mkdirSync(output,{recursive:true});
const input=join(output,'input.svg');
writeFileSync(input,readFileSync(source,'utf8').replace('../../src/assets/brand-mark.png',`data:image/png;base64,${mark}`));
const cli=join(root,'node_modules/@tauri-apps/cli/tauri.js');
const icons=join(root,'src-tauri/icons');
const desktopFiles=readdirSync(icons).filter(file=>/\.(png|ico|icns)$/.test(file));
execFileSync(process.execPath,[cli,'icon',input,'--output',output],{cwd:root,stdio:'inherit',windowsHide:true});
for(const file of desktopFiles) {
  if(!existsSync(join(output,file)))throw Error(`Generator did not produce ${file}`);
  copyFileSync(join(output,file),join(icons,file));
}
copyFileSync(join(output,'icon.png'),join(root,'assets/app-icon/source.png'));
console.log(`Updated ${desktopFiles.length} desktop icons from the About mark on a white tile.`);
