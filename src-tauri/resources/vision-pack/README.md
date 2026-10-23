# Vision recognition pack (bundled with installer)

Staging directory for first-run seed into `%LOCALAPPDATA%\xmu-sports\vision-packs\`.

```text
manifest.json
health.ok
templates/page_rules.json
```

Regenerate (updates version from `package.json` + placeholder signature):

```bat
powershell -File scripts\ensure_vision_pack_bundle.ps1
```

Shell templates live in the binary (`bundled_shell_templates`); this pack only gates native run + page rules.
