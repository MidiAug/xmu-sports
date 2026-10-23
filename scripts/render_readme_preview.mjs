import { mkdirSync, copyFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

// Every screen is rendered from test fixtures. Never connects to the desktop host.
const output=resolve('.github/assets');
const staging=resolve('artifacts/readme-demo');
mkdirSync(output,{recursive:true});
const result=spawnSync(process.execPath,['scripts/test_history_availability_ui.mjs'],{
  stdio:'inherit',env:{...process.env,XMU_UI_ARTIFACTS:staging}
});
if(result.status!==0) throw new Error('Demo rendering failed; no README images replaced');
copyFileSync(resolve(staging,'partial.png'),resolve(output,'booking-demo.png'));
copyFileSync(resolve(staging,'history.png'),resolve(output,'history-demo.png'));
writeFileSync(resolve(output,'hero.svg'),`<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="320" viewBox="0 0 1200 320" role="img" aria-labelledby="title desc">
<title id="title">XMU Sports</title><desc id="desc">A clear view of your next booking. Windows desktop app.</desc>
<defs><linearGradient id="bg" x2="1" y2="1"><stop stop-color="#f0f5ff"/><stop offset="1" stop-color="#fafaf7"/></linearGradient></defs>
<rect width="1200" height="320" rx="24" fill="url(#bg)"/>
<g font-family="Segoe UI,Arial,sans-serif"><text x="64" y="78" font-size="13" letter-spacing="3" fill="#617391">WINDOWS DESKTOP</text><text x="62" y="151" font-size="56" font-weight="650" fill="#172b49">XMU Sports</text><text x="65" y="197" font-size="21" fill="#536783">A clear view of your next booking.</text>
<g font-size="13" fill="#49617e"><rect x="64" y="239" width="88" height="30" rx="15" fill="#e5ecf6"/><text x="85" y="259">Tauri 2</text><rect x="163" y="239" width="85" height="30" rx="15" fill="#e5ecf6"/><text x="184" y="259">Rust</text><rect x="259" y="239" width="126" height="30" rx="15" fill="#e5ecf6"/><text x="279" y="259">TypeScript</text></g></g>
<g transform="translate(760,52)"><rect x="0" y="0" width="354" height="216" rx="18" fill="#fff" stroke="#dce5ef"/><path d="M0 44H354" stroke="#e9edf3"/><circle cx="24" cy="22" r="4" fill="#b2c2d5"/><circle cx="39" cy="22" r="4" fill="#d0dae6"/><circle cx="54" cy="22" r="4" fill="#e0e6ed"/>
<rect x="24" y="70" width="200" height="12" rx="6" fill="#dfe8f5"/><rect x="24" y="95" width="142" height="8" rx="4" fill="#edf1f6"/>
<g fill="#f1f5fa" stroke="#e1e8f2"><rect x="24" y="126" width="92" height="36" rx="8"/><rect x="128" y="126" width="92" height="36" rx="8"/><rect x="232" y="126" width="98" height="36" rx="8"/></g>
<path d="M34 189H229" stroke="#e8edf4" stroke-width="6" stroke-linecap="round"/><path d="m292 185 7 7 13-15" stroke="#659e85" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>
</g></svg>\n`);
console.log('README images generated entirely from code and synthetic data');
