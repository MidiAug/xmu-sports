# XMU Sports

[![Checks](https://github.com/MidiAug/xmu-sports/actions/workflows/checks.yml/badge.svg)](https://github.com/MidiAug/xmu-sports/actions/workflows/checks.yml)

厦大体育场馆预约辅助桌面应用，面向 Windows 10/11。使用 Tauri 2、TypeScript、Rust 和 Python HTTP 执行器。非学校官方应用，仅用于本人或已授权账号，请遵守学校的预约规则。

## 功能

- 配置运动、场馆、日期、时段和场地，支持定时任务及 0–300 秒执行延迟。
- 查询今天和明天的场次。管理员锁定与人数已满分开显示：全锁时段不可选，部分锁定标记具体场地，已满仍可选。
- 场次按账号、场馆和实际日期持久缓存，重启及跨天可复用；手动刷新更新快照，正式预约另查实时余量。
- 凭证过期后可单独重新登录，成功后自动刷新场次；这个入口不执行预约。
- 历史按阶段展示执行过程、时间和耗时；上游错误回复经脱敏后保存在本地诊断中。
- 提交响应不明时先查单确认，无法确认则保留待确认状态，避免盲目重下。

## 使用

需要 PC 微信和 WebView2。安装包包含 HTTP 执行器及必要验证码分类模型，使用者无需安装 Python。可从 [Releases](https://github.com/MidiAug/xmu-sports/releases) 获取已发布版本；若尚无版本，请使用开发模式。

首次使用需要在微信小程序完成登录。程序尝试使用 Windows UI Automation 恢复登录且不移动鼠标；微信没有提供可操作元素时会提示手动完成。最小化或完全遮挡的小程序可能无法自动操作，不能保证后台登录成功。登录完成后会尝试关闭本次打开的小程序窗口。

学校数据与登录方式可能变化；人数是查询时快照，不能保证最终预约成功。任务只在应用运行时执行，请保持电脑唤醒及网络连接。

## 开发模式

准备 Node.js 22、Rust stable（MSVC 工具链）、Python 3.12 x64、Git，以及 Visual Studio 的“使用 C++ 的桌面开发”和 WebView2。

```powershell
npm ci
npm run ensure-http-bundle
npm run tauri:dev
```

首次准备执行器会创建项目内 Python 环境并下载依赖。平时调整界面继续运行 `tauri:dev`，无需修改版本号或制作安装包。只看前端可运行 `npm run dev`；没有 Tauri 宿主时不能执行真实预约。

## 检查与构建

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml -p xmu-sports --lib
cd src-tauri/sidecars/http-booking
.venv/Scripts/python.exe -m pip install -r requirements-dev.txt
.venv/Scripts/python.exe -m pytest -q
```

上述宿主检查需要先准备执行器资源。单元测试使用模拟数据，不应触发真实预约。浏览器交互、安装升级及真实微信登录需要另外验证，详见 [贡献说明](CONTRIBUTING.md)。

确需生成 Windows NSIS 安装包时执行 `npm run tauri:build`，输出位于 `src-tauri/target/release/bundle/nsis/`。安装新版前退出应用，安装器保留个人任务配置和历史。

## 源码结构

| 路径 | 用途 |
| --- | --- |
| `src/` | 页面、配置、历史与场次状态 |
| `src-tauri/src/` | Tauri 宿主、调度、登录与诊断 |
| `src-tauri/sidecars/http-booking/` | 正式 HTTP 执行器、模型及离线测试 |
| `scripts/` | 检查与资源构建脚本 |
| `src-tauri/vision-runtime/` | 历史视觉工作区，保留编译依赖关系，不作为当前产品执行入口 |

公开仓库不包含内部研究记录、AI 协作规则、用户数据库、会话、原始抓包和现场截图。请勿在 Issue 中粘贴 Cookie、完整票据、个人信息或未检查的日志包。

[更新记录](CHANGELOG.md)　[贡献说明](CONTRIBUTING.md)　[CI/CD](.github/CI_CD.md)　[第三方说明](THIRD_PARTY_NOTICES.md)

## 许可证

项目自身代码使用 [MIT License](LICENSE)，第三方模型和依赖保留各自许可证。
