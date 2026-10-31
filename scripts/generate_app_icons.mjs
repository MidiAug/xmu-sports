// Reuse the About/header mark on the application's white rounded tile.
import {readFileSync,writeFileSync,mkdirSync,readdirSync,copyFileSync,existsSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {execFileSync} from 'node:child_process';
const root=resolve(import.meta.dirname,'..');
const source=join(root,'assets/app-icon/source.svg');
const mark=readFileSync(join(root,'src/assets/brand-mark.svg'),'utf8');
const [x,y,width,height]=mark.match(/viewBox="([^"]+)"/)[1].split(/\s+/).map(Number);
const tile=`<rect x="${x}" y="${y}" width="${width}" height="${height}" rx="${width*224/1024}" fill="#ffffff"/>`;
writeFileSync(source,mark.replace(/(<svg\b[^>]*>)/,`$1\n${tile}`));
const output=join(root,'artifacts/app-icon-generated');
mkdirSync(output,{recursive:true});
const cli=join(root,'node_modules/@tauri-apps/cli/tauri.js');
const icons=join(root,'src-tauri/icons');
const desktopFiles=readdirSync(icons).filter(file=>/\.(png|ico|icns)$/.test(file));
execFileSync(process.execPath,[cli,'icon',source,'--output',output],{cwd:root,stdio:'inherit',windowsHide:true});
for(const file of desktopFiles) {
  if(!existsSync(join(output,file)))throw Error(`Generator did not produce ${file}`);
  copyFileSync(join(output,file),join(icons,file));
}
copyFileSync(join(output,'icon.png'),join(root,'assets/app-icon/source.png'));
console.log(`Updated ${desktopFiles.length} desktop icons from the About mark on a white tile.`);
