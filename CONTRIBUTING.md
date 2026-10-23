# 贡献与开发

请先阅读 README 的开发模式说明。通常只需运行开发模式，无需升级版本或构建安装包。

## 修改与验证

- 前端逻辑：`npm test`；类型和静态资源：`npm run build`。
- Rust 宿主：`cargo test --manifest-path src-tauri/Cargo.toml -p xmu-sports --lib`。
- HTTP 执行器：在其目录使用 Python 3.12 环境运行 `python -m pytest -q`；测试依赖见 requirements-dev.txt，包含历史通道模拟测试所需 websockets 与单测超时保护。
- 浏览器交互：先构建前端，配置 `PLAYWRIGHT_MODULE` 为 Playwright 模块路径，再执行 `npm run test:select` 或相关 `scripts/test_*_ui.mjs`。需要安装 Edge，部分脚本有开发机路径回退，应显式覆盖。
- 安装升级检查：`npm run test:installer`，需要先有一次本机 Tauri NSIS 构建生成工具和辅助文件；测试只执行隔离的安装判断函数。
- 打包审计：准备已打包执行器后，使用执行器 Python 环境运行 `scripts/test_lite_bundle.py`。

离线检查、微信只读登录和真实预约是不同层次。提交 PR 时说明实际执行的检查；普通检查不得提交真实预约。

## 数据与诊断

不要提交真实 Cookie、票据、账号数据、个人数据库、原始抓包或原始诊断。日志虽会脱敏，分享前仍应人工检查。新增错误处理要保留定位信息，并确保秘密与个人信息不会进入公开输出。提交响应不明时应查单，不能自动重复提交。

内部协作规则和开发笔记仅保留在开发者本地；公共贡献规范以本文件为准。历史视觉 Python 的现场截图未公开，相关旧测试不作为当前验收入口。历史 Rust 视觉代码仍是 Cargo 工作区成员，删除目录前需处理工作区及测试引用。

## 版本

日常修改写入 CHANGELOG 的 Unreleased。准备发行时同步 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 与 `src-tauri/tauri.conf.json` 的应用版本，并补充对应版本更新记录。CI/CD 使用说明见 [.github/CI_CD.md](.github/CI_CD.md)。


## 生成公开示意图

先运行 `npm run build`，在另一个终端启动 `npm run preview -- --host 127.0.0.1 --port 1421`，然后执行 `node scripts/render_readme_preview.mjs`。可用 `XMU_TEST_BASE_URL` 指定其他本地端口；Playwright 依赖按前述 `PLAYWRIGHT_MODULE` 配置。

脚本通过假 Tauri 边界和固定示例数据渲染页面，并进行交互验收，再将图片写入 `.github/assets/`。不会访问桌面宿主、微信或学校接口。不要用真实账号截图替换这些公开示意图。

只读历史性能测量可设置 `XMU_HISTORY_BENCH_ROOT` 为本机 runs 目录，然后运行 `cargo test --manifest-path src-tauri/Cargo.toml -p xmu-sports --lib benchmark_history_read -- --ignored --nocapture`，仅输出记录数量及冷读、缓存读取耗时。
